use crate::Result;
use crate::pwm::Pwm;
use crate::gpio::{Gpio, OutputPin};

pub struct Motor {
    pwm: Pwm,
    forward_enable: OutputPin,
    backward_enable: OutputPin,
}

const PWM_WAVE_PERIOD_NS: u32 = 2_000_000;

impl Motor {
    pub fn new(gpio: &Gpio, channel: u32, forward_enable: u8, backward_enable: u8) -> Result<Self> {
        let pwm = Pwm::new(0, channel, PWM_WAVE_PERIOD_NS)?;
        let forward_enable = gpio.get(forward_enable)?
            .into_output_low();
        let backward_enable = gpio.get(backward_enable)?
            .into_output_low();

        Ok(Self {
            pwm,
            forward_enable,
            backward_enable,
        })
    }

    /// Set speed in the range [-1.0, 1.0], with negative numbers spinning backwards.
    pub fn set_speed(&mut self, speed: f32) {
        if speed > 0.0 {
            eprintln!("setting motor to positive speed");
            self.backward_enable.set_low();
            self.forward_enable.set_high();
        } else if speed < 0.0 {
            self.forward_enable.set_low();
            self.backward_enable.set_high();
        } else {
            // enable break mode
            self.backward_enable.set_low();
            self.forward_enable.set_low();
        }
        let duty_cycle = PWM_WAVE_PERIOD_NS as f32 * speed.abs();
        let _ = self.pwm.pwm.set_duty_cycle_ns(duty_cycle as _);
    }
}

impl Drop for Motor {
    fn drop(&mut self) {
        let _ = self.pwm.pwm.set_duty_cycle_ns(0);
        self.forward_enable.set_low();
        self.backward_enable.set_low();
    }
}

pub mod ports {
    //! This module groups ports by whether they're for left or right motors, because each motor
    //! controller board controls a different side of the robot.

    /// Use channel 1: GPIO 13, physical header pin 33
    pub const FRONT_LEFT_PWM_CHANNEL: u32 = 1;
    /// Use channel 3: GPIO 19, physical header pin 35
    pub const BACK_LEFT_PWM_CHANNEL: u32 = 3;
    /// Use channel 2: GPIO 18, physical header pin 12
    pub const FRONT_RIGHT_PWM_CHANNEL: u32 = 2;
    /// Use channel 0: GPIO 12, physical header pin 32
    pub const BACK_RIGHT_PWM_CHANNEL: u32 = 0;

    /// Physical header pin 29
    pub const FRONT_LEFT_FORWARD_ENABLE: u8 = 5;
    /// Physical header pin 31
    pub const FRONT_LEFT_BACKWARD_ENABLE: u8 = 6;
    /// Physical header pin 36
    pub const BACK_LEFT_FORWARD_ENABLE: u8 = 16;
    /// Physical header pin 37
    pub const BACK_LEFT_BACKWARD_ENABLE: u8 = 26;
    /// Physical header pin 11
    pub const FRONT_RIGHT_FORWARD_ENABLE: u8 = 17;
    /// Physical header pin 13
    pub const FRONT_RIGHT_BACKWARD_ENABLE: u8 = 27;
    /// Physical header pin 16
    pub const BACK_RIGHT_FORWARD_ENABLE: u8 = 23;
    /// Physical header pin 18
    pub const BACK_RIGHT_BACKWARD_ENABLE: u8 = 24;
}
