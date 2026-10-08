use std::path::Path;

use linux_embedded_hal::I2cdev;
use embedded_hal::i2c::ErrorType;
use hiwonder_encoder_motor_module::{MotorType, set_motors_type};

pub const FRONT_LEFT_MOTOR_PORT: u8 = 0;
pub const BACK_LEFT_MOTOR_PORT: u8 = 1;
pub const FRONT_RIGHT_MOTOR_PORT: u8 = 2;
pub const BACK_RIGHT_MOTOR_PORT: u8 = 3;

pub fn init_motors<P: AsRef<Path>>(dev_path: P, ty: MotorType) -> Result<I2cdev, <I2cdev as ErrorType>::Error> {
    let mut i2c = I2cdev::new(dev_path)?;
    set_motors_type(&mut i2c, ty)?;
    Ok(i2c)
}
