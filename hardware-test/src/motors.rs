use anyhow::{Context, Result};
use hardware::motor::{Motor, ports};
use clap::{Parser, ValueEnum};
use rppal::gpio::Gpio;

#[derive(ValueEnum, Copy, Clone, Debug, PartialEq, Eq)]
#[value()]
enum MotorName {
    FrontLeft,
    FrontRight,
    BackLeft,
    BackRight,
}

#[derive(Parser, Clone, Debug)]
pub struct MotorsArgs {
    /// Speed to set the motors to
    ///
    /// The value provided will be clamped to the range [-1.0, 1.0], and negative values run the
    /// motors in reverse.
    #[arg(short = 's', long, allow_hyphen_values = true, default_value_t = 0.0)]
    speed: f32,

    /// Wheels to spin (use the flag multiple times to run multiple wheels)
    #[arg(short = 'w', long, value_enum,
        default_values_t = [MotorName::FrontLeft, MotorName::FrontRight, MotorName::BackLeft, MotorName::BackRight])]
    wheels: Vec<MotorName>,
}

pub struct MotorsTest {
    args: MotorsArgs,
    fl_drive_motor: Motor,
    bl_drive_motor: Motor,
    fr_drive_motor: Motor,
    br_drive_motor: Motor,
}

impl MotorsTest {
    pub fn new(mut args: MotorsArgs) -> Result<Self> {
        args.speed = args.speed.clamp(-1.0, 1.0);

        let gpio = Gpio::new()
            .context("Failed to open GPIO")?;
        let fl_drive_motor = Motor::new(
            &gpio,
            ports::FRONT_LEFT_PWM_CHANNEL,
            ports::FRONT_LEFT_FORWARD_ENABLE,
            ports::FRONT_LEFT_BACKWARD_ENABLE,
        ).context("Failed to initialize front-left motor")?;
        let bl_drive_motor = Motor::new(
            &gpio,
            ports::BACK_LEFT_PWM_CHANNEL,
            ports::BACK_LEFT_FORWARD_ENABLE,
            ports::BACK_LEFT_BACKWARD_ENABLE,
        ).context("Failed to initialize back-left motor")?;
        let fr_drive_motor = Motor::new(
            &gpio,
            ports::FRONT_RIGHT_PWM_CHANNEL,
            ports::FRONT_RIGHT_FORWARD_ENABLE,
            ports::FRONT_RIGHT_BACKWARD_ENABLE,
        ).context("Failed to initialize front-right motor")?;
        let br_drive_motor = Motor::new(
            &gpio,
            ports::BACK_RIGHT_PWM_CHANNEL,
            ports::BACK_RIGHT_FORWARD_ENABLE,
            ports::BACK_RIGHT_BACKWARD_ENABLE,
        ).context("Failed to initialize back-right motor")?;

        return Ok(Self {
            args,
            fl_drive_motor,
            bl_drive_motor,
            fr_drive_motor,
            br_drive_motor,
        })
    }

    pub fn start(&mut self) {
        if self.args.wheels.contains(&MotorName::FrontLeft) {
            self.fl_drive_motor.set_speed(self.args.speed);
        }
        if self.args.wheels.contains(&MotorName::BackLeft) {
            self.bl_drive_motor.set_speed(self.args.speed);
        }
        if self.args.wheels.contains(&MotorName::FrontRight) {
            self.fr_drive_motor.set_speed(self.args.speed);
        }
        if self.args.wheels.contains(&MotorName::BackRight) {
            self.br_drive_motor.set_speed(self.args.speed);
        }
    }
}
