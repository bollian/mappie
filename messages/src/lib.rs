use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Clone, Copy, Debug, PartialEq)]
pub struct Move {
    pub translate: mint::Vector2<f32>,
    pub rotate: f32,
}

impl Move {
    // f32s are always encoded as u32s (no length variation) on the wire
    // TODO: calculate the actual overhead of COBS encoding + termination
    pub const POSTCARD_COBS_BUFFER_MAX_SIZE: usize = std::mem::size_of::<Move>() * 2 + 1;

    pub fn stop() -> Self {
        Self {
            translate: mint::Vector2::from([0.0, 0.0]),
            rotate: 0.0,
        }
    }
}

#[derive(Deserialize, Serialize, Clone, Copy, Debug, PartialEq)]
pub struct SensorReading {

}
