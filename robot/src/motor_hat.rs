// use embedded_hal::i2c::{I2c, SevenBitAddress};
//
// pub struct MotorHat<I: I2c> {
//     i2c_bus: I,
// }
//
// impl<I: I2c> MotorHat<I> {
//     pub fn new(i2c_bus: I) {
//         Self {
//             i2c_bus
//         }
//     }
//
//     /// Throttle in range [-1.0, 1.0]. A throttle of 0.0 engages brake mode.
//     pub fn throttle_motor1(throttle: f32) {
//         let throttle = throttle.clamp(-1.0, 1.0);
//
//         
//     }
// }
//
// struct I2cPin<I: I2c> {
//     i2c_bus: &mut I,
//     address: SevenBitAddress,
// }
