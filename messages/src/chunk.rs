use std::io::{self, Result};
use std::net::SocketAddr;
use std::sync::Arc;
use std::ops::ControlFlow;

use tokio::net::UdpSocket;

/// Maximum size of a status message chunk.
const CHUNK_SIZE: usize = 512;

/// Size of the protocol header at the start of a status message chunk.
const HEADER_SIZE: usize = 4;

struct Header {
    frame_identifier: u16,
    chunk_identifier: u16,
    continuation_flag: bool,
}

impl Header {
    fn to_network_bytes(&self) -> [u8; 4] {
        let [first, second] = self.frame_identifier.to_be_bytes();
        let second_half = if self.continuation_flag {
            self.chunk_identifier | CONTINUATION_FLAG
        } else {
            self.chunk_identifier & !CONTINUATION_FLAG
        };
        let [third, fourth] = second_half.to_be_bytes();

        [first, second, third, fourth]
    }

    fn from_network_bytes(bytes: [u8; 4]) -> Self {
        let frame_identifier = u16::from_be_bytes([bytes[0], bytes[1]]);
        let mut chunk_identifier = u16::from_be_bytes([bytes[2], bytes[3]]);
        let continuation_flag = (chunk_identifier & CONTINUATION_FLAG) != 0;
        chunk_identifier &= !CONTINUATION_FLAG;

        Self {
            frame_identifier,
            chunk_identifier,
            continuation_flag,
        }
    }
}

/// Amount of serialized data that can fit into a single status message
///
/// ```text
/// +------------------+-------------------+------------------+
/// |      16 bits     |       1 bit       |     15 bits      |
/// +------------------+-------------------+------------------+
/// | Frame Identifier | Continuation Flag | Chunk Identifier |
/// +------------------+-------------------+------------------+
/// ```
const CHUNK_DATA_SIZE: usize = CHUNK_SIZE - HEADER_SIZE;

const CONTINUATION_FLAG: u16 = 0b1000_0000_0000_0000;

/// The absolute maximum number of chunks that a single frame can be split into.
///
/// This value is true because the chunk identifier is 15 bits, so we take that maximum value and
/// add one for the 0th chunk.
const MAX_CHUNK_COUNT: usize = 0b1000_0000_0000_0000;

const fn min_chunk_count_for_data(serialization_buf_size: usize) -> usize {
    let count = serialization_buf_size.div_ceil(CHUNK_DATA_SIZE);
    if count == 0 {
        // zero-sized messages can still send a single (empty) datagram
        return 1
    }
    count
}

#[derive(Clone, Copy, Default)]
struct ChunkTracker {
    received: bool,
    buffer_index: usize,
    buffer_len: usize,
}

pub struct ChunkedReceiver {
    pub(crate) socket: Arc<UdpSocket>, // can be shared between sender and receiver

    /// Expected frame identifier for the next received chunk
    ///
    /// Receiving a different frame identifier is grounds for ignoring that chunk or advancing this
    /// counter, depending on the exact value received.
    next_frame: u16,

    /// Where received data is saved in chunk form before final assembly
    chunk_buffers: Vec<[u8; CHUNK_SIZE]>,

    /// Index of each item in this list is the chunk identifier for that tracker
    chunk_trackers: Vec<ChunkTracker>,

    /// The highest number chunk we've received
    ///
    /// This chunk will be marked as received in its chunk tracker unless we haven't actually
    /// received any chunks for the current frame. This field isn't strictly necessary, but allows
    /// us to avoid iterating the entire tracker list as an optimization in some situations.
    last_chunk: u16,

    /// Where received data is assembled from its chunks
    assembly_buf: Vec<u8>,

    /// Number of dropped packets
    dropped_packets: usize,

    /// Number of frames we know we've missed
    missed_frames: usize,

    /// Number of frames we've successfully received
    received_frames: usize,

    /// Number of chunks we've successfully received
    received_chunks: usize,
}

impl ChunkedReceiver {
    pub fn new(socket: Arc<UdpSocket>, max_serialization_size: usize) -> Self {
        let min_chunk_count = min_chunk_count_for_data(max_serialization_size);
        if min_chunk_count > MAX_CHUNK_COUNT {
            panic!("Deserialization buffer too large");
        }

        Self {
            socket,
            next_frame: 0,
            chunk_buffers: Vec::with_capacity(min_chunk_count),
            chunk_trackers: vec![Default::default(); min_chunk_count],
            last_chunk: 0,
            assembly_buf: Vec::with_capacity(max_serialization_size),
            dropped_packets: 0,
            missed_frames: 0,
            received_frames: 0,
            received_chunks: 0,
        }
    }

    pub fn dropped_chunks(&self) -> usize {
        self.dropped_packets
    }

    pub fn missed_frames(&self) -> usize {
        self.missed_frames
    }

    pub fn received_frames(&self) -> usize {
        self.received_frames
    }

    pub fn received_chunks(&self) -> usize {
        self.received_chunks
    }

    pub async fn receive(&mut self) -> Result<(&[u8], SocketAddr)> {
        self.reset_frame(0);
        while self.chunk_buffers.len() < self.chunk_buffers.capacity() {
            let chunk = self.chunk_buffers.push_mut([0; _]);
            let (size, addr) = match self.socket.recv_from(chunk.as_mut_slice()).await {
                Ok((size, addr)) => (size, addr),
                Err(e) => match e.kind() {
                    io::ErrorKind::Interrupted => continue,
                    _ => return Err(e)
                },
            };

            let received = &chunk[..size];
            let Some((header, _data)) = received.split_first_chunk() else {
                // received chunk was too small
                self.chunk_buffers.pop();
                self.dropped_packets += 1;
                continue
            };
            let header = Header::from_network_bytes(*header);

            // We must consider the possibility that the frame identifier hit u16::MAX and wrapped.
            // If the wrapping distance to the received frame identifier is less than half u16::MAX
            // (but not zero), it means the identifier is not too far ahead of us and we consider it
            // as being most likely that the sender is ahead of us. If the wrapping distance is more
            // than half u16::MAX, the likeliest explanation is that we are ahead of a long-delayed
            // packet.
            let wrapping_distance = header.frame_identifier.wrapping_sub(self.next_frame);
            if wrapping_distance != 0 {
                // we have received the incorrect frame
                if wrapping_distance > u16::MAX / 2 {
                    // this chunk is too old, drop it
                    self.chunk_buffers.pop();
                    continue
                } else {
                    // treat this as the first received chunk of the next frame
                    if self.chunk_buffers.len() > 1 {
                        self.chunk_buffers[0] = *self.chunk_buffers.last().unwrap();
                    }
                    self.missed_frames = self.missed_frames.saturating_add(wrapping_distance as usize);
                    self.next_frame = header.frame_identifier;
                    self.reset_frame(1);
                }
            }

            if let Err(()) = self.track_received_chunk(header.chunk_identifier, size) {
                // chunk count was too large, move on to the next frame
                self.next_frame = self.next_frame.wrapping_add(1);
                self.missed_frames = self.missed_frames.saturating_add(1);
                self.reset_frame(0);
                continue
            }

            // we check if the message is ready every time just in case of out-of-order packets
            if self.message_ready() {
                self.next_frame = self.next_frame.wrapping_add(1); // mark this frame as received
                self.received_frames = self.received_frames.saturating_add(1);
                self.build_message().expect("Unable to build ready message");
                return Ok((self.assembly_buf.as_slice(), addr));
            }
        }

        Err(io::Error::new(io::ErrorKind::OutOfMemory, "Received data frame was too large"))
    }

    fn reset_frame(&mut self, saved_chunks: usize) {
        self.chunk_trackers[..=self.last_chunk as usize].fill(ChunkTracker::default());
        self.chunk_buffers.truncate(saved_chunks);
        self.last_chunk = 0;
    }

    fn track_received_chunk(&mut self, chunk_id: u16, content_len: usize) -> std::result::Result<(), ()> {
        if let Some(tracker) = self.chunk_trackers.get_mut(chunk_id as usize) {
            if tracker.received {
                // This chunk was already received. The latest packet is a duplicate, and should be
                // dropped.
                self.chunk_buffers.pop();
                return Ok(())
            }

            tracker.received = true;
            tracker.buffer_index = self.chunk_buffers.len() - 1;
            tracker.buffer_len = content_len;
            self.last_chunk = self.last_chunk.max(chunk_id);

            Ok(())
        } else { Err(()) }
    }

    /// Iterate the received chunks in order until a gap is reached.
    ///
    /// Returns true if the last chunk was reached, false otherwise.
    fn iter_chunks<F>(trackers: &[ChunkTracker], buffers: &[[u8; CHUNK_SIZE]], mut f: F) -> bool
    where
        F: FnMut(Header, &[u8]) -> ControlFlow<()>
    {
        let mut continuation_flag = true;
        for tracker in trackers.iter() {
            if tracker.received {
                let (header, data) = buffers[tracker.buffer_index][..tracker.buffer_len].split_first_chunk()
                    .expect("It should not be possible for the chunk tracker to point to a too-small chunk");
                let header = Header::from_network_bytes(*header);
                continuation_flag = header.continuation_flag;
                match f(header, data) {
                    ControlFlow::Break(_) => break,
                    ControlFlow::Continue(_) => {}
                }
            } else { break }
        }

        !continuation_flag
    }

    fn message_ready(&self) -> bool {
        Self::iter_chunks(&self.chunk_trackers, &self.chunk_buffers, |header, _| if header.continuation_flag {
            ControlFlow::Continue(())
        } else {
            ControlFlow::Break(())
        })
    }

    fn build_message(&mut self) -> Option<()> {
        self.assembly_buf.clear();

        let mut received_chunks = 0;
        let all_received = Self::iter_chunks(&self.chunk_trackers, &self.chunk_buffers, |header, data| {
            received_chunks += 1;
            self.assembly_buf.extend(data);

            if header.continuation_flag {
                ControlFlow::Continue(())
            } else {
                ControlFlow::Break(())
            }
        });

        if !all_received {
            return None;
        }

        self.received_chunks = self.received_chunks.saturating_add(received_chunks);
        Some(())
    }
}

#[derive(Debug)]
pub struct ChunkedSender {
    next_identifier: u16,
    tx_buf: Vec<u8>,
    socket: Arc<UdpSocket>,
}

impl ChunkedSender {
    pub fn new(socket: Arc<UdpSocket>, max_serialization_size: usize) -> Self {
        let min_chunk_count = min_chunk_count_for_data(max_serialization_size);
        if min_chunk_count > MAX_CHUNK_COUNT {
            panic!("Serialization buffer too large");
        }

        Self {
            next_identifier: 0,
            tx_buf: vec![0; HEADER_SIZE + max_serialization_size],
            socket,
        }
    }

    pub fn serialization_buf(&mut self) -> &mut [u8] {
        &mut self.tx_buf[HEADER_SIZE..]
    }

    pub async fn send(&mut self, payload_len: usize) -> Result<()> {
        if payload_len > self.tx_buf.len() - HEADER_SIZE {
            let desc = format!("Payload was larger than configured {} byte limit", self.tx_buf.len() - HEADER_SIZE);
            return Err(io::Error::new(io::ErrorKind::OutOfMemory, desc));
        }

        let identifier = self.next_identifier;

        // make sure we don't get cancelled without updating this
        self.next_identifier = self.next_identifier.wrapping_add(1);

        let chunk_count = min_chunk_count_for_data(payload_len);
        let mut scratch_buf = &mut self.tx_buf[..payload_len + HEADER_SIZE];
        for chunk_number in 0..chunk_count {
            let (end, continuation_flag) = if scratch_buf.len() <= CHUNK_SIZE {
                (scratch_buf.len(), false)
            } else {
                (CHUNK_SIZE, true)
            };
            let header = Header {
                frame_identifier: identifier,
                chunk_identifier: chunk_number as u16,
                continuation_flag,
            }.to_network_bytes();
            let (msg_header, _) = scratch_buf.split_first_chunk_mut()
                .expect("Writing header into impossibly small message buffer");
            *msg_header = header;

            self.socket.send(&scratch_buf[..end]).await?;
            scratch_buf = &mut scratch_buf[(end - HEADER_SIZE)..];
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn sender_receiver() -> (ChunkedSender, ChunkedReceiver) {
        const BUF_SIZE: usize = CHUNK_SIZE * 2;

        let receiver = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let receiver_addr = receiver.local_addr().unwrap();
        let receiver = ChunkedReceiver::new(receiver, BUF_SIZE);

        let sender = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        sender.connect(receiver_addr).await.unwrap();
        let sender = ChunkedSender::new(sender, BUF_SIZE);

        (sender, receiver)
    }

    #[tokio::test]
    async fn send_single_chunk() {
        let (mut sender, mut receiver) = sender_receiver().await;
        let payload = [1, 2, 3];
        sender.serialization_buf()[..payload.len()].copy_from_slice(&payload);
        sender.send(payload.len()).await.unwrap();
        assert_eq!(receiver.receive().await.unwrap().0, payload);
    }

    #[tokio::test]
    async fn send_empty_payload() {
        let (mut sender, mut receiver) = sender_receiver().await;
        sender.send(0).await.unwrap();
        assert_eq!(receiver.receive().await.unwrap().0, Vec::<u8>::new());
    }

    #[tokio::test]
    async fn send_full_chunk() {
        let data = vec![0; CHUNK_DATA_SIZE];
        let (mut sender, mut receiver) = sender_receiver().await;
        sender.serialization_buf()[..data.len()].copy_from_slice(&data);
        sender.send(data.len()).await.unwrap();
        assert_eq!(receiver.receive().await.unwrap().0, data);
    }

    /// Send a message that takes slightly more than one frame
    #[tokio::test]
    async fn send_slightly_large_payload() {
        let data = vec![0; CHUNK_DATA_SIZE + 1];
        let (mut sender, mut receiver) = sender_receiver().await;
        sender.serialization_buf()[..data.len()].copy_from_slice(&data);
        sender.send(data.len()).await.unwrap();
        assert_eq!(receiver.receive().await.unwrap().0, data);
        assert_eq!(receiver.received_chunks(), 2);
    }
}
