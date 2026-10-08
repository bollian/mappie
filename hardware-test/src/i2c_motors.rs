use std::ffi::OsString;
use std::path::Path;
use anyhow::{Context, Result};
use hiwonder_encoder_motor_module::{MotorType, set_pwms};
use hardware::hiwonder_motor_board::init_motors;
use linux_embedded_hal::I2cdev;

use clap::Parser;

/// Test motors using the HiWonder I2C motor controller board
#[derive(Parser, Clone, Debug)]
pub struct I2cMotorsArgs {
    /// Path to the device file for the I2C bus with the motor controller board
    #[arg(long, short, default_value_os_t = OsString::from("/dev/i2c-1"))]
    dev: OsString,

    /// List of motor ports to drive
    #[arg(long, short, default_values_t = [0, 1, 2, 3])]
    motors: Vec<u8>,

    /// Speed to set the motors to
    ///
    /// The value provided will be clamped to the range [-1.0, 1.0], and negative values run the
    /// motors in reverse.
    #[arg(long, short, default_value_t = 0.0)]
    speed: f32,
}

pub struct I2cMotorsTest {
    args: I2cMotorsArgs,
    i2c: I2cdev,
}

impl I2cMotorsTest {
    pub fn new(args: I2cMotorsArgs) -> Result<Self> {
        if args.speed < -1.0 || args.speed > 1.0 {
            anyhow::bail!("Motor speed must be in the range [-1.0, 1.0]");
        }

        let i2c = init_motors(Path::new(args.dev.as_os_str()), MotorType::NoEncoder)
            .with_context(|| format!("Failed to open I2c device \"{}\"", args.dev.display()))?;
        Ok(Self {
            args,
            i2c,
        })
    }

    pub fn start(&mut self) -> Result<()> {
        // set_motors_type(&mut self.i2c, MotorType::NoEncoder)
        //     .context("Failed to set motor type")?;
        let mut pwms = [0; 4];
        for &port in self.args.motors.iter() {
            pwms[port as usize] = (self.args.speed * 100.0).round() as i8;
        }
        set_pwms(&mut self.i2c, pwms)
            .context("Failed to set motor speeds")
    }
}

impl Drop for I2cMotorsTest {
    fn drop(&mut self) {
        if let Err(e) = set_pwms(&mut self.i2c, [0, 0, 0, 0]) {
            log::error!("Failed to stop I2C motors during cleanup: {}", e);
        }
    }
}
