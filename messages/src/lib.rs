pub mod chunk;

use std::fmt::{Display, Formatter};
use std::io::{self, Result};
use std::marker::PhantomData;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::net::UdpSocket;
use tokio::task::JoinSet;
use tokio::time::timeout;

use chunk::{ChunkedSender, ChunkedReceiver};

pub struct PostcardSender<T: Serialize> {
    chunked: ChunkedSender,
    _phantom: PhantomData<T>,
}

impl<T: Serialize> PostcardSender<T> {
    pub fn new(socket: Arc<UdpSocket>, max_serialization_size: usize) -> Self {
        Self {
            chunked: ChunkedSender::new(socket, max_serialization_size),
            _phantom: PhantomData,
        }
    }

    pub async fn send(&mut self, msg: &T) -> Result<()> {
        let serialized_len = {
            let serialization_buf = self.chunked.serialization_buf();
            match postcard::to_slice(msg, serialization_buf) {
                Ok(slice) => slice.len(),
                Err(postcard::Error::SerializeBufferFull) => {
                    let desc = format!("Serialized message was larger than configured {} byte limit", serialization_buf.len());
                    return Err(io::Error::new(io::ErrorKind::OutOfMemory, desc));
                },
                Err(e) => panic!("Unexpected serialization error ({e})"),
            }
        };

        self.chunked.send(serialized_len).await
    }
}

pub struct PostcardReceiver<T: for<'a> Deserialize<'a>> {
    chunked: ChunkedReceiver,
    _phantom: PhantomData<T>,
}

impl<T: for<'a> Deserialize<'a>> PostcardReceiver<T> {
    pub fn new(socket: Arc<UdpSocket>, max_serialization_size: usize) -> Self {
        Self {
            chunked: ChunkedReceiver::new(socket, max_serialization_size),
            _phantom: PhantomData,
        }
    }

    pub fn dropped_chunks(&self) -> usize { self.chunked.dropped_chunks() }
    pub fn missed_frames(&self) -> usize { self.chunked.missed_frames() }
    pub fn received_frames(&self) -> usize { self.chunked.received_frames() }
    pub fn received_chunks(&self) -> usize { self.chunked.received_chunks() }

    pub async fn receive(&mut self) -> Result<(T, SocketAddr)> {
        let (serialized, addr) = self.chunked.receive().await?;
        let msg = postcard::from_bytes(serialized)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        Ok((msg, addr))
    }
}

pub type StatusSender = PostcardSender<StatusMessage>;
pub type StatusReceiver = PostcardReceiver<StatusMessage>;

#[derive(Deserialize, Serialize, Clone, Copy, Debug, PartialEq)]
pub enum ControlMessage {
    EstablishControl, // a greeting message, after which the controller waits for a reply
    Move(Move),
}

impl ControlMessage {
    // f32s are always encoded as u32s (no length variation) on the wire. The extra headroom is just
    // being defensive.
    pub const BUF_SIZE: usize = std::mem::size_of::<Self>() * 2 + 1;
}

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub enum StatusMessage {
    SensorReading(SensorReading),
}

impl StatusMessage {
    /// Expected maximum size of serialized message
    ///
    /// We can get at most 1450 lidar points from a single scan, and each point should serialize to
    /// no more than 10 bytes (8 bytes of actual data, plus 2 bytes for the varint overhead of the
    /// larger fields). An additional 5 bytes is set aside for the 32-bit discard count.
    pub const SERIALIZATION_BUF_SIZE: usize = 1450 * 10 + 5;
}

#[derive(Deserialize, Serialize, Clone, Copy, Debug, PartialEq)]
pub struct Move {
    pub translate: mint::Vector2<f32>,
    pub rotate: f32,
}

impl Display for Move {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "Move ({}, {})↻{}", self.translate.x, self.translate.y, self.rotate)
    }
}

impl Move {
    pub fn stop() -> Self {
        Self {
            translate: mint::Vector2::from([0.0, 0.0]),
            rotate: 0.0,
        }
    }
}

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct SensorReading {
    pub scan: hardware::lidar::Scan,
}

/// A channel for sending control messages to the robot
///
/// This is intended for use the operator interface, whereas [`StatusSender`] is intended for the
/// robot.
#[derive(Debug)]
pub struct ControlSender {
    socket: Arc<UdpSocket>,
}

impl ControlSender {
    pub async fn control(&self, msg: ControlMessage) -> Result<()> {
        let mut buf = [0; ControlMessage::BUF_SIZE];
        let msg_bytes = match postcard::to_slice_cobs(&msg, &mut buf[..]) {
            Ok(slice) => slice,
            Err(e) => panic!("Unexpected serialization error ({e}) on message: {msg:?}"),
        };

        self.socket.send(msg_bytes).await?;
        Ok(())
    }

    pub async fn wait_until_writable(&self) -> Result<()> {
        self.socket.writable().await
    }
}

/// Wait for a connection from an operator.
///
/// # Panics
///
/// This method panics if given an empty list of addresses.
pub async fn listen_for_operator<A: Iterator<Item = SocketAddr>>(addrs: A) -> Result<(StatusSender, ControlReceiver)> {
    let mut join_set = JoinSet::new();
    for addr in addrs {
        join_set.spawn(async move {
            let socket = UdpSocket::bind(addr).await?;
            loop {
                // listen for new connections
                if let Ok((ControlMessage::EstablishControl, addr)) = ControlReceiver::receive_from(&socket).await {
                    socket.connect(addr).await?;
                    let socket = Arc::new(socket);
                    let sender = StatusSender::new(Arc::clone(&socket), StatusMessage::SERIALIZATION_BUF_SIZE);
                    let receiver = ControlReceiver {
                        socket,
                    };
                    return Ok((sender, receiver))
                }
            }
        });
    }

    let res = join_set.join_next().await
        .expect("Listen address list was empty");
    match res {
        Ok(res) => res,
        Err(join_err) => std::panic::resume_unwind(join_err.into_panic()),
    }
}

pub async fn establish_control(dest_addr: SocketAddr) -> Result<(ControlSender, StatusReceiver)> {
    let mut buf = [0; ControlMessage::BUF_SIZE];
    let retry_period = Duration::from_secs(5);

    let bind_addr = match dest_addr.ip() {
        IpAddr::V4(_) => ("0.0.0.0", 0),
        IpAddr::V6(_) => ("::", 0),
    };
    log::trace!("Binding out-going socket to {bind_addr:?}");
    let socket = Arc::new(UdpSocket::bind(bind_addr).await?);
    log::trace!("Connecting to {dest_addr}");
    socket.connect(dest_addr).await?;
    let sender = ControlSender { socket };

    // wait until we receive a message in reply
    loop {
        log::trace!("Attempting to establish control");
        sender.control(ControlMessage::EstablishControl).await?;
        log::trace!("Waiting for control acknowledgement from robot");
        match timeout(retry_period, sender.socket.peek(&mut buf[..])).await {
            Ok(res) => res?,
            Err(_) => continue, // timeout
        };

        log::trace!("Controller socket connected");
        let receiver = StatusReceiver::new(Arc::clone(&sender.socket), StatusMessage::SERIALIZATION_BUF_SIZE);
        return Ok((sender, receiver))
    }

}

/// A channel for receiving control messages from the operator.
///
/// This is intended for use by the robot in tandem with a simultaneously constructed
/// [`StatusSender`]. The operator uses a [`ControlChannel`] to send messages to this.
#[derive(Debug)]
pub struct ControlReceiver {
    socket: Arc<UdpSocket>,
}

impl ControlReceiver {
    pub async fn receive(&self) -> Result<ControlMessage> {
        Self::receive_from(&self.socket).await
            .map(|(msg, _addr)| msg)
    }

    async fn receive_from(socket: &UdpSocket) -> Result<(ControlMessage, SocketAddr)> {
        let mut buf = [0; ControlMessage::BUF_SIZE];
        let (n, addr) = socket.recv_from(&mut buf[..]).await?;
        let msg = postcard::from_bytes_cobs(&mut buf[..n])
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        Ok((msg, addr))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    async fn sender_receiver<T: Serialize + for<'a> Deserialize<'a>>() -> (PostcardSender<T>, PostcardReceiver<T>) {
        const BUF_SIZE: usize = 512 * 2;

        let receiver = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let receiver_addr = receiver.local_addr().unwrap();
        let receiver = PostcardReceiver::new(receiver, BUF_SIZE);

        let sender = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        sender.connect(receiver_addr).await.unwrap();
        let sender = PostcardSender::new(sender, BUF_SIZE);

        (sender, receiver)
    }

    #[tokio::test]
    async fn send_simple_message() {
        let (mut sender, mut receiver) = sender_receiver().await;
        sender.send(&true).await.unwrap();
        assert!(receiver.receive().await.unwrap().0);
    }

    #[tokio::test]
    async fn send_empty_message() {
        let (mut sender, mut receiver) = sender_receiver().await;
        sender.send(&()).await.unwrap();
        receiver.receive().await.unwrap();
    }

    #[tokio::test]
    async fn send_normal_message() {
        #[derive(Deserialize, Serialize, Debug, PartialEq, Eq)]
        struct NormalMessage {
            first: u8,
            second: u16,
            third: String,
        }

        let data = NormalMessage {
            first: 1,
            second: 2,
            third: "hello world".to_string(),
        };
        let (mut sender, mut receiver) = sender_receiver().await;
        sender.send(&data).await.unwrap();
        assert_eq!(receiver.receive().await.unwrap().0, data);
    }

    #[tokio::test]
    async fn send_full_message() {
        let data = vec![0; 508 - 2];
        let (mut sender, mut receiver) = sender_receiver().await;
        sender.send(&data).await.unwrap();
        assert_eq!(receiver.receive().await.unwrap().0, data);
    }

    #[tokio::test]
    async fn send_slightly_large_message() {
        let data = vec![0; 508];
        let (mut sender, mut receiver) = sender_receiver().await;
        sender.send(&data).await.unwrap();
        assert_eq!(receiver.receive().await.unwrap().0, data);
        assert_eq!(receiver.received_chunks(), 2);
    }
}
