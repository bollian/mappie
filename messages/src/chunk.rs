use std::io::{self, Result};
use std::marker::PhantomData;
use std::net::SocketAddr;
use std::sync::Arc;
use std::ops::ControlFlow;

use serde::{Deserialize, Serialize};
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

pub struct ChunkedReceiver<T: for<'a> Deserialize<'a>> {
    pub(crate) socket: Arc<UdpSocket>, // can be shared between sender and receiver

    /// Expected frame identifier for the next received chunk
    ///
    /// Receiving a different frame identifier is grounds for ignoring that chunk or advancing this
    /// counter, depending on the exact value received.
    next_frame: u16,

    /// Where received data is saved in chunk form before final assembly into
    /// `serialization_scratch_buf`
    chunk_buffers: Vec<[u8; CHUNK_SIZE]>,

    /// Index of each item in this list is the chunk identifier for that tracker
    chunk_trackers: Vec<ChunkTracker>,

    /// The highest number chunk we've received
    ///
    /// This chunk will be marked as received in its chunk tracker unless we haven't actually
    /// received any chunks for the current frame. This field isn't strictly necessary, but allows
    /// us to avoid iterating the entire tracker list as an optimization in some situations.
    last_chunk: u16,

    /// Where received data is copied to before final deserialization
    serialization_scratch_buf: Vec<u8>,

    /// Number of dropped packets
    dropped_packets: usize,

    /// Number of frames we know we've missed
    missed_frames: usize,

    /// Number of frames we've successfully received
    received_frames: usize,

    _phantom: PhantomData<T>,
}

impl<T: for<'a> Deserialize<'a>> ChunkedReceiver<T> {
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
            serialization_scratch_buf: Vec::with_capacity(max_serialization_size),
            dropped_packets: 0,
            missed_frames: 0,
            received_frames: 0,
            _phantom: Default::default(),
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

    pub async fn receive(&mut self) -> Result<(T, SocketAddr)> {
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
                return self.build_message().expect("Unable to build ready message")
                    .map(|msg| (msg, addr))
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e));
            }
        }

        return Err(io::Error::new(io::ErrorKind::OutOfMemory, "Received data frame was too large"));
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

        return !continuation_flag
    }

    fn message_ready(&self) -> bool {
        Self::iter_chunks(&self.chunk_trackers, &self.chunk_buffers, |header, _| if header.continuation_flag {
            ControlFlow::Continue(())
        } else {
            ControlFlow::Break(())
        })
    }

    fn build_message(&mut self) -> Option<postcard::Result<T>> {
        // start with a fresh deserialization buffer
        self.serialization_scratch_buf.clear();

        let all_received = Self::iter_chunks(&self.chunk_trackers, &self.chunk_buffers, |header, data| {
            self.serialization_scratch_buf.extend(data);

            if header.continuation_flag {
                ControlFlow::Continue(())
            } else {
                ControlFlow::Break(())
            }
        });

        if !all_received {
            return None;
        }

        Some(postcard::from_bytes(self.serialization_scratch_buf.as_slice()))
    }
}

#[derive(Debug)]
pub struct ChunkedSender<T: Serialize> {
    next_identifier: u16,
    tx_buf: Vec<u8>,
    socket: Arc<UdpSocket>,
    _phantom: PhantomData<T>,
}

impl<T: Serialize> ChunkedSender<T> {
    pub fn new(socket: Arc<UdpSocket>, max_serialization_size: usize) -> Self {
        let min_chunk_count = min_chunk_count_for_data(max_serialization_size);
        if min_chunk_count > MAX_CHUNK_COUNT {
            panic!("Serialization buffer too large");
        }

        Self {
            next_identifier: 0,
            tx_buf: vec![0; HEADER_SIZE + max_serialization_size],
            socket,
            _phantom: Default::default(),
        }
    }

    pub async fn update(&mut self, msg: &T) -> Result<()> {
        let identifier = self.next_identifier;

        // make sure we don't get cancelled without updating this
        self.next_identifier = self.next_identifier.wrapping_add(1);

        let serialized = match postcard::to_slice(msg, &mut self.tx_buf[HEADER_SIZE..]) {
            Ok(s) => s,
            Err(postcard::Error::SerializeBufferFull) => {
                let desc = format!("Serialized message was larger than configured {} limit", self.tx_buf.capacity());
                return Err(io::Error::new(io::ErrorKind::OutOfMemory, desc))
            },
            Err(e) => panic!("Unexpected serialization error ({e})"),
        };

        let serialized_end = serialized.len() + HEADER_SIZE;
        let mut content_index = 0;
        let mut chunk_number = 0;
        while content_index < serialized_end {
            let end = (content_index + CHUNK_SIZE).min(serialized_end);
            let header = Header {
                frame_identifier: identifier,
                chunk_identifier: chunk_number,
                continuation_flag: end != serialized_end
            }.to_network_bytes();

            let chunk_content = &mut self.tx_buf[content_index..end];
            let (msg_header, _) = chunk_content.split_first_chunk_mut()
                .expect("Accessing header out of impossibly small message buffer");
            *msg_header = header;

            self.socket.send(chunk_content).await?;

            content_index += CHUNK_DATA_SIZE;
            chunk_number += 1;
        }

        return Ok(())
    }
}
