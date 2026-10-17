use embedded_hal::i2c::ErrorType;
use hardware::hiwonder_motor_board::*;
use hiwonder_encoder_motor_module::{MotorType, set_pwms};
use linux_embedded_hal::I2cdev;
use messages::Move;

use crate::config::Config;

pub struct Drive {
    conf: crate::Conf,
    i2c: I2cdev,
}

impl Drive {
    /// Initialize the drive system.
    ///
    /// Provide the I2c bus used for communicating with the HiWonder motor driver board.
    pub fn new(conf: crate::Conf) -> Result<Self, <I2cdev as ErrorType>::Error> {
        let i2c = init_motors(conf.i2c_dev_path.as_path(), MotorType::NoEncoder)?;
        Ok(Self {
            conf,
            i2c,
        })
    }

    pub fn control(&mut self, control: Move) {
        let Move { translate, rotate } = control;

        fn deadzone(val: f32, zone: f32) -> f32 {
            let zone = zone.abs();
            if val < zone && val > -zone {
                return 0.0
            }
            val
        }

        // deadzones prevent motors being sent power that's too low to actually move the bot, which
        // would be a stall scenario and could lead to excess power draw, noise, and motor wear
        let x = deadzone(translate.x, self.conf.deadzone);
        let y = deadzone(translate.y, self.conf.deadzone);
        let rot = deadzone(rotate, self.conf.deadzone);

        let speeds = mecanum_wheel_speeds(x, y, rot);
        if let Err(e) = self.set_speeds(speeds) {
            log::error!("Failed to set motor speeds: {e}");
        }
    }

    /// Set the speeds of the wheels in the range [-1.0, 1.0], with negatives spinning backwards
    ///
    /// You don't need to pass in negative values to account for motor orientation. That is handled
    /// internally by looking at the hardware configuration.
    fn set_speeds(&mut self, speeds: Speeds) -> Result<(), <I2cdev as ErrorType>::Error> {
        let conf = Config::get_global();
        let mut pwms = [0; 4];

        pwms[FRONT_LEFT_MOTOR_PORT as usize] = (speeds.fl * conf.fl_max * 100.0) as i8;
        pwms[BACK_LEFT_MOTOR_PORT as usize] = (speeds.bl * conf.bl_max * 100.0) as i8;
        pwms[FRONT_RIGHT_MOTOR_PORT as usize] = (speeds.fr * conf.fr_max * 100.0) as i8;
        pwms[BACK_RIGHT_MOTOR_PORT as usize] = (speeds.br * conf.br_max * 100.0) as i8;
        for pwm in pwms.iter_mut() {
            *pwm = (*pwm).clamp(-100, 100);
        }
        log::debug!("Setting wheel speeds {pwms:?}");
        set_pwms(&mut self.i2c, pwms)
    }

    pub fn stop(&mut self) -> Result<(), <I2cdev as ErrorType>::Error> {
        set_pwms(&mut self.i2c, [0; 4])
    }
}

impl Drop for Drive {
    fn drop(&mut self) {
        if let Err(e) = self.stop() {
            log::error!("Failed to stop drive motors during shutdown: {e}");
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Speeds {
    /// Front-Left
    fl: f32,

    /// Back-Left
    bl: f32,

    /// Front-Right
    fr: f32,

    /// Back-Right
    br: f32,
}

fn mecanum_wheel_speeds(vx: f32, vy: f32, rot: f32) -> Speeds {
    let mut fl = vy + vx + rot;
    let mut bl = vy - vx + rot;
    let mut fr = vy - vx - rot;
    let mut br = vy + vx - rot;

    // scale the speeds down if we find ourselves trying to go more than 100% on a single wheel
    let div = fl.abs()
        .max(bl.abs())
        .max(fr.abs())
        .max(br.abs());
    if div > 1.0 {
        fl /= div;
        bl /= div;
        fr /= div;
        br /= div;
    }

    return Speeds { fl, bl, fr, br };
}
