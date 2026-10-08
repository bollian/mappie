use embedded_hal::i2c::ErrorType;
use hardware::hiwonder_motor_board::*;
use hiwonder_encoder_motor_module::{MotorType, set_pwms};
use linux_embedded_hal::I2cdev;
use messages::Move;
use nalgebra::Vector3;

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

        // Scale control down to a unit vector if we're being told to do something we're uncapable
        // of. This will happen if the user instructs us to move 100% forwards AND sideways
        // simultaneously, for example.
        // let v = Vector3::new(x, y, rot);
        // let norm = v.norm();
        // let v = if norm > 1.0 {
        //     v.unscale(norm)
        // } else { v };

        // let speeds = mecanum_wheel_speeds(self.conf.wheelbase_width, self.conf.wheelbase_length, x, y, rot);
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

// fn mecanum_wheel_speeds(wheelbase_width: f32, wheelbase_length: f32, vx: f32, vy: f32, rot: f32) -> Speeds {
//     // shape parameter of the robot
//     let t = (wheelbase_width + wheelbase_length) / 2.0;
//
//     let w = 4. * rot.powi(2);
//     let w = w + 3. * vx.powi(2);
//     let w = w + 3. * vy.powi(2);
//     let w = (w * t.powi(2)) - rot.powi(2);
//     let w = w.sqrt();
//
//     let fl = (t * vx) + (t * vy) - rot - w;
//     let fr = (-t * vx) + (t * vy) + rot - w;
//     let bl = (-t * vx) + (t * vy) - rot + w;
//     let br = (t * vx) + (t * vy) + rot + w;
//
//     let fl = fl / 4. / t;
//     let fr = fr / 4. / t;
//     let bl = bl / 4. / t;
//     let br = br / 4. / t;
//
//     return Speeds { fl, fr, bl, br };
// }

fn mecanum_wheel_speeds(vx: f32, vy: f32, rot: f32) -> Speeds {
    let fl = vy + vx + rot;
    let bl = vy - vx + rot;
    let fr = vy - vx - rot;
    let br = vy + vx - rot;

    let fl = fl.clamp(-1.0, 1.0);
    let bl = bl.clamp(-1.0, 1.0);
    let fr = fr.clamp(-1.0, 1.0);
    let br = br.clamp(-1.0, 1.0);

    return Speeds { fl, bl, fr, br };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_forward_speeds() {
        let speeds = mecanum_wheel_speeds(10.0, 10.0, 0.0, 1.0, 0.0);
        assert_eq!(speeds.fl, 1.0);
        assert_eq!(speeds.bl, 1.0);
        assert_eq!(speeds.fr, 1.0);
        assert_eq!(speeds.br, 1.0);
    }
}
