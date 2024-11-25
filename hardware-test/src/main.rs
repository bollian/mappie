use std::ffi::OsString;

use adafruit_motorkit::dc::DcMotor;
use clap::{Parser, ValueEnum};
use linux_embedded_hal as hal;
use eyre::*;

#[derive(ValueEnum, Copy, Clone, Debug)]
#[value()]
enum Motor {
    FrontLeft,
    FrontRight,
    BackLeft,
    BackRight,
}

#[derive(Parser, Clone, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Speed to set the motors to
    ///
    /// The value provided will be clamped to the range [-1.0, 1.0], and negative values run the
    /// motors in reverse.
    #[arg(short = 's', long, default_value_t = 1.0)]
    speed: f32,

    /// Path to the I2C bus device file with an attached PWM controller
    #[arg(short = 'd', long, default_value = "/dev/i2c-1")]
    i2c_dev: OsString,

    /// Address of the PCA9685 PWM controller on the I2C bus
    #[arg(short = 'a', long, default_value_t = 0x60)]
    pwm_addr: u8,

    // /// PCA9685 prescalar that determines the PWM frequency
    // #[arg(short = 'p', long, default_value_t = 4)]
    // prescale: u8,

    /// Wheels to spin
    #[arg(value_enum, default_values_t = [Motor::FrontLeft, Motor::FrontRight, Motor::BackLeft, Motor::BackRight])]
    wheels: Vec<Motor>,
}

fn main() -> Result<()> {
    env_logger::init();

    let mut args = Args::parse();
    args.speed = args.speed.clamp(-1.0, 1.0);

    let dev = hal::I2cdev::new(args.i2c_dev.as_os_str())
        .wrap_err("Failed to open I2C device")?;
    // let mut pwm = pwm_pca9685::Pca9685::new(dev, args.pwm_addr)
    //     .map_err(|e| eyre::eyre!("Failed to construct pwm device: {:?}", e))?;
    // pwm.enable()
    //     .map_err(|e| eyre!("Failed to enable PWM: {:?}", e))?;
    // pwm.set_prescale(args.prescale)
    //     .map_err(|e| eyre!("Failed to set PWM prescale: {:?}", e))?;
    let mut pwm = adafruit_motorkit::init_pwm(Some(dev))
        .wrap_err("Failed to initialize PWM controller on motor hat")?;

    let mut fl_drive_motor = DcMotor::try_new(&mut pwm, ports::FL_DRIVE_MOTOR)
        .wrap_err("unable to construct front left motor")?;
    let mut fr_drive_motor = DcMotor::try_new(&mut pwm, ports::FR_DRIVE_MOTOR)
        .wrap_err("unable to construct front right motor")?;
    let mut bl_drive_motor = DcMotor::try_new(&mut pwm, ports::BL_DRIVE_MOTOR)
        .wrap_err("unable to construct back left motor")?;
    let mut br_drive_motor = DcMotor::try_new(&mut pwm, ports::BR_DRIVE_MOTOR)
        .wrap_err("unable to construct back right motor")?;

    for wheel in args.wheels {
        let motor = match wheel {
            Motor::FrontLeft => &mut fl_drive_motor,
            Motor::FrontRight => &mut fr_drive_motor,
            Motor::BackLeft => &mut bl_drive_motor,
            Motor::BackRight => &mut br_drive_motor,
        };

        motor.set_throttle(&mut pwm, args.speed)
            .wrap_err("Issue setting speed of motor")?;
    }

    println!("Hit Enter to stop the test...");
    let mut line = String::new();
    let _ = std::io::stdin().read_line(&mut line);

    let _ = fl_drive_motor.set_throttle(&mut pwm, 0.0);
    let _ = fr_drive_motor.set_throttle(&mut pwm, 0.0);
    let _ = bl_drive_motor.set_throttle(&mut pwm, 0.0);
    let _ = br_drive_motor.set_throttle(&mut pwm, 0.0);

    return Ok(());
}

mod ports {
    pub const BL_DRIVE_MOTOR: adafruit_motorkit::Motor = adafruit_motorkit::Motor::Motor1;
    pub const BR_DRIVE_MOTOR: adafruit_motorkit::Motor = adafruit_motorkit::Motor::Motor2;
    pub const FL_DRIVE_MOTOR: adafruit_motorkit::Motor = adafruit_motorkit::Motor::Motor3;
    pub const FR_DRIVE_MOTOR: adafruit_motorkit::Motor = adafruit_motorkit::Motor::Motor4;
}
