pub mod chunk;

use std::fmt::{Display, Formatter};
use std::io::{self, Result};
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::net::UdpSocket;
use tokio::task::JoinSet;
use tokio::time::timeout;

use chunk::{ChunkedSender, ChunkedReceiver};

pub struct PostcardSender<T: Serialize>(ChunkedSender);

impl<T: Serialize> PostcardSender<T> {
    pub fn new(socket: Arc<UdpSocket>, max_serialization_size: usize) -> Self {
        Self(ChunkedSender::new(socket, max_serialization_size))
    }

    pub fn send(&mut self) -> Result<()> {
        todo!()
    }
}

pub struct PostcardReceiver(ChunkedReceiver);

impl<T: for<'a> Deserialize<'a>> PostcardReceiver<T> {
    pub fn new(socket: Arc<UdpSocket>, max_serialization_size: usize) -> Self {
        Self(ChunkedReceiver::new(socket, max_serialization_size))
    }

    pub fn dropped_chunks(&self) -> usize { self.0.dropped_chunks() }
    pub fn missed_frames(&self) -> usize { self.0.missed_frames() }
    pub fn received_frames(&self) -> usize { self.0.received_frames() }
    pub fn received_chunks(&self) -> usize { self.0.received_chunks() }

    pub fn receive(&mut self) -> Result<(T, SocketAddr)> {
        todo!()
    }
}

pub type StatusSender = ChunkedSender<StatusMessage>;
pub type StatusReceiver = ChunkedReceiver<StatusMessage>;

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
                match ControlReceiver::receive_from(&socket).await {
                    Ok((msg, addr)) if msg == ControlMessage::EstablishControl => {
                        socket.connect(addr).await?;
                        let socket = Arc::new(socket);
                        let sender = StatusSender::new(Arc::clone(&socket), StatusMessage::SERIALIZATION_BUF_SIZE);
                        let receiver = ControlReceiver {
                            socket,
                        };
                        return Ok((sender, receiver))
                    }
                    _ => {}
                }
            }
        });
    }

    let res = join_set.join_next().await
        .expect("Listen address list was empty");
    match res {
        Ok(res) => return res,
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
