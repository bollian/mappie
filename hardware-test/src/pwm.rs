use std::time::Instant;

use anyhow::{Context, Result, bail};
use clap::Parser;
use hardware::pwm::{Pwm, self};

/// Increase, then decrease, the duty cycle of PWM channels in a sawtooth wave
#[derive(Parser, Clone, Debug)]
pub struct PwmArgs {
    /// Length of the sawtooth wave in seconds
    #[arg(short, long, default_value_t = 5.0)]
    length: f32,

    /// Period of the PWM signal in nanoseconds
    #[arg(short = 'P', long, default_value_t = 10000)]
    period: u32,

    /// Which PWM chip to use channels from
    #[arg(short, long, default_value_t = 0)]
    chip: u32,

    /// Which PWM channels to test
    #[arg(short, long, default_values_t = [0, 1, 2, 3])]
    pwms: Vec<u32>,
}

pub struct PwmTest {
    args: PwmArgs,
    pwms: Vec<Pwm>,
    start_timestamp: Instant,
}

impl PwmTest {
    pub fn new(args: PwmArgs) -> Result<Self> {
        if args.length.is_sign_negative() {
            bail!("PWM cycle period must be positive");
        }

        let mut pwms = Vec::with_capacity(args.pwms.len());
        for &channel in &args.pwms {
            if !pwm::chip_channel_exists(args.chip, channel) {
                bail!("PWM chip does not contain channel {channel}");
            }

            let pwm = Pwm::new(args.chip, channel, args.period)
                .with_context(|| format!("Failed to open sysfs PWM channel {channel} on chip {}", args.chip))?;
            pwms.push(pwm);
        }

        Ok(Self {
            args,
            pwms,
            start_timestamp: Instant::now(),
        })
    }

    pub fn start(&mut self) -> Result<()> {
        Ok(())
    }

    pub fn periodic(&mut self) -> Result<()> {
        let test_duration = Instant::now() - self.start_timestamp;
        let time_shift = test_duration.as_secs_f32() % self.args.length;
        let scale = 2.0 * time_shift / self.args.length;
        let scale = if scale < 1.0 {
            // scale is in the first half of the sawtooth, so it scales upward
            scale
        } else {
            // scale is in the second half of the sawtooth, so it scales downward
            2.0 - scale
        };

        // loop through the PWMs, setting their duty cycle to `scale * period`
        for pwm in &self.pwms {
            let period = pwm.pwm.get_period_ns()
                .context("Unable to get PWM period")?;
            let duty_cycle = period as f32 * scale;
            pwm.pwm.set_duty_cycle_ns(duty_cycle as u32)
                .context("Unable to set PWM duty cycle")?;
        }

        Ok(())
    }
}
