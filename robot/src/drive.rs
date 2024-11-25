use crate::Pwm;

use adafruit_motorkit::dc::DcMotor;
use eyre::{Result, WrapErr};
use messages::Move;

pub struct Drive {
    args: crate::Args,
    fl_drive_motor: DcMotor,
    fr_drive_motor: DcMotor,
    bl_drive_motor: DcMotor,
    br_drive_motor: DcMotor,
}

impl Drive {
    pub fn new(args: crate::Args, pwm: &mut Pwm) -> Result<Self> {
        let fl_drive_motor = DcMotor::try_new(pwm, ports::FL_DRIVE_MOTOR)
            .wrap_err("unable to construct front left motor")?;
        let fr_drive_motor = DcMotor::try_new(pwm, ports::FR_DRIVE_MOTOR)
            .wrap_err("unable to construct front right motor")?;
        let bl_drive_motor = DcMotor::try_new(pwm, ports::BL_DRIVE_MOTOR)
            .wrap_err("unable to construct back left motor")?;
        let br_drive_motor = DcMotor::try_new(pwm, ports::BR_DRIVE_MOTOR)
            .wrap_err("unable to construct back right motor")?;

        Ok(Self {
            args,
            fl_drive_motor,
            fr_drive_motor,
            bl_drive_motor,
            br_drive_motor,
        })
    }

    pub fn main_loop(&mut self, pwm: &mut Pwm, control: Move) {
        let Move { translate, rotate } = control;

        fn deadzone(val: f32, zone: f32) -> f32 {
            let zone = zone.abs();
            if val < zone && val > -zone {
                return 0.0
            }
            val
        }

        let translate = mint::Vector2 {
            x: deadzone(translate.x, self.args.deadzone),
            y: deadzone(translate.y, self.args.deadzone),
        };
        let rotate = deadzone(rotate, self.args.deadzone);

        let fl_speed = (translate.y + translate.x + rotate).clamp(-1.0, 1.0);
        let fr_speed = (translate.y - translate.x - rotate).clamp(-1.0, 1.0);
        let bl_speed = (translate.y - translate.x + rotate).clamp(-1.0, 1.0);
        let br_speed = (translate.y + translate.x - rotate).clamp(-1.0, 1.0);

        self.fl_drive_motor.set_throttle(pwm, -fl_speed).expect("FL drive throttle failed");
        self.fr_drive_motor.set_throttle(pwm, fr_speed).expect("FR drive throttle failed");
        self.bl_drive_motor.set_throttle(pwm, -bl_speed).expect("BL drive throttle failed");
        self.br_drive_motor.set_throttle(pwm, br_speed).expect("BR drive throttle failed");
    }

    pub fn reset(&mut self, pwm: &mut Pwm) {
        let _ = self.fl_drive_motor.set_throttle(pwm, 0.0);
        let _ = self.fr_drive_motor.set_throttle(pwm, 0.0);
        let _ = self.bl_drive_motor.set_throttle(pwm, 0.0);
        let _ = self.br_drive_motor.set_throttle(pwm, 0.0);
    }
}

mod ports {
    pub const BL_DRIVE_MOTOR: adafruit_motorkit::Motor = adafruit_motorkit::Motor::Motor1;
    pub const BR_DRIVE_MOTOR: adafruit_motorkit::Motor = adafruit_motorkit::Motor::Motor2;
    pub const FL_DRIVE_MOTOR: adafruit_motorkit::Motor = adafruit_motorkit::Motor::Motor3;
    pub const FR_DRIVE_MOTOR: adafruit_motorkit::Motor = adafruit_motorkit::Motor::Motor4;
}
