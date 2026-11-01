use std::ffi::OsString;
use std::path::Path;
use std::time::Duration;
use anyhow::{Context, Result};
use hiwonder_encoder_motor_module::{MotorType, set_pwms, set_closed_loop_speeds, set_motors_type, encoder_totals};
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
    #[arg(long, short, value_delimiter = ',', default_values_t = [0, 1, 2, 3])]
    motors: Vec<u8>,

    /// Speed to set the motors to
    ///
    /// The value provided will be clamped to the range [-1.0, 1.0], and negative values run the
    /// motors in reverse.
    #[arg(long, short, allow_negative_numbers = true, default_value_t = 0.0)]
    speed: f32,

    /// Use 12V encoder motors
    #[arg(long = "12v", default_value_t = false)]
    v12: bool,

    /// Speed in encoder clicks per 10ms. Negative values run the motor in reverse.
    #[arg(long, short, allow_negative_numbers = true)]
    clicks: Option<i8>,
}

pub struct I2cMotorsTest {
    args: I2cMotorsArgs,
    i2c: I2cdev,
    previous_totals: Option<[u32; 4]>,
}

impl I2cMotorsTest {
    pub fn new(args: I2cMotorsArgs) -> Result<Self> {
        if args.speed < -1.0 || args.speed > 1.0 {
            anyhow::bail!("Motor speed must be in the range [-1.0, 1.0]");
        }

        let ty = if args.v12 {
            MotorType::JGB37
        } else { MotorType::NoEncoder };

        let i2c = init_motors(Path::new(args.dev.as_os_str()), ty)
            .with_context(|| format!("Failed to open I2c device \"{}\"", args.dev.display()))?;
        Ok(Self {
            args,
            i2c,
            previous_totals: None,
        })
    }

    pub fn start(&mut self) -> Result<()> {
        if let Some(clicks) = self.args.clicks {
            set_closed_loop_speeds(&mut self.i2c, [clicks; _])
        } else {
            let pwm = (self.args.speed * 100.0).round() as i8;
            set_pwms(&mut self.i2c, [pwm; _])
        }.context("Failed to set motor speeds")
    }

    pub fn periodic(&mut self, delta_time: Duration) -> Result<()> {
        let totals = encoder_totals(&mut self.i2c)
            .context("Failed to get encoder totals")?;

        let speeds = if delta_time.is_zero() {
            None
        } else {
            self.previous_totals.map(|previous| {
                std::array::from_fn::<_, 4, _>(|i| {
                    totals[i].wrapping_sub(previous[i]) as i32 as f64
                        * 0.01
                        / delta_time.as_secs_f64()
                })
            })
        };
        self.previous_totals = Some(totals);

        if let Some(speeds) = speeds {
            println!(
                "{} us; encoder totals: {:?}; speed (clicks/10ms): [{:.2}, {:.2}, {:.2}, {:.2}]",
                delta_time.as_micros(),
                totals.map(|total| total as i32),
                speeds[0],
                speeds[1],
                speeds[2],
                speeds[3],
            );
        } else {
            println!(
                "{} us; encoder totals: {:?}; speed (clicks/10ms): n/a",
                delta_time.as_micros(),
                totals.map(|total| total as i32),
            );
        }

        Ok(())
    }
}

impl Drop for I2cMotorsTest {
    fn drop(&mut self) {
        let _ = set_motors_type(&mut self.i2c, MotorType::NoEncoder); // disable closed-loop control
        if let Err(e) = set_pwms(&mut self.i2c, [0; _]) {
            log::error!("Failed to stop I2C motors during cleanup: {}", e);
        }
    }
}
