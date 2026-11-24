mod i2c_motors;
mod lidar;
mod pwm;

use std::io::Read;
use std::num::NonZeroU32;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use rustix::event::{PollFd, PollFlags, poll};
use rustix::time::Timespec;

use i2c_motors::*;
use lidar::*;
use pwm::*;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[command(subcommand)]
    subcommand: Command,

    /// Skip initializing the GPIO pins, which could result in some connected hardware not working
    #[arg(long, default_value_t = false)]
    skip_gpio: bool,

    /// Don't prompt for a safe exit (prevents the process from stopping when run in the background)
    #[arg(long, default_value_t = false)]
    no_prompt: bool,

    /// Delay between periodic updates, in milliseconds
    #[arg(long, default_value = "20")]
    period_ms: NonZeroU32,
}

#[derive(Subcommand)]
enum Command {
    Lidar(LidarArgs),
    Pwm(PwmArgs),
    I2cMotors(I2cMotorsArgs),
}

fn main() -> Result<()> {
    env_logger::init();

    let args = Args::parse();
    let mut lidar = None;
    let mut pwm = None;
    let mut i2c_motors = None;
    match args.subcommand {
        Command::Lidar(args) => {
            lidar = Some(LidarTest::new(args)
                .context("Failed to initialize lidar test")?);
        }
        Command::Pwm(args) => {
            pwm = Some(PwmTest::new(args)
                .context("Failed to initialize PWM test")?);
        }
        Command::I2cMotors(args) => {
            i2c_motors = Some(I2cMotorsTest::new(args)
                .context("Failed to initialize I2c motors test")?);
        }
    }

    if let Some(pwm) = &mut pwm {
        pwm.start()
            .context("Failed to start PWM test")?;
    }
    if let Some(i2c_motors) = &mut i2c_motors {
        i2c_motors.start()
            .context("Failed to start I2c motors test")?;
    }

    if !args.no_prompt {
        // wait for the user to hit enter, periodically calling into the test so it can print
        // things or do other periodic work
        println!("Hit Enter to stop the test...");
    }

    let period = Duration::from_millis(args.period_ms.get().into());
    let mut stdin = std::io::stdin();
    let mut stdin_buf = [0; 256];
    let mut previous_periodic = Instant::now();
    let mut next_start = previous_periodic + period;
    loop {
        let remaining = next_start.saturating_duration_since(Instant::now());
        if !remaining.is_zero() {
            std::thread::sleep(remaining);
        }

        if !args.no_prompt {
            let poll_timeout = Timespec {
                tv_sec: 0,
                tv_nsec: 0,
            };
            let poll_count = poll(
                &mut [PollFd::new(&stdin, PollFlags::IN)],
                Some(&poll_timeout),
            )
            .context("Stdin poll operation failed")?;
            if poll_count == 1 {
                let count = stdin.read(&mut stdin_buf[..])?;
                if count == 0 || stdin_buf[..count].contains(&b'\n') {
                    break;
                }
                stdin_buf[..count].fill(0);
            }
        }

        let scheduled_start = next_start;
        let now = Instant::now();
        let delta_time = now.duration_since(previous_periodic);
        previous_periodic = now;

        if let Some(pwm_test) = &mut pwm {
            pwm_test.periodic()?;
        }
        if let Some(lidar_test) = &mut lidar {
            lidar_test.periodic()?;
        }
        if let Some(i2c_motors_test) = &mut i2c_motors {
            i2c_motors_test.periodic(delta_time)?;
        }

        next_start = next_iteration_start(scheduled_start, period);
    }

    anyhow::Result::<()>::Ok(())
}

fn next_iteration_start(scheduled_start: Instant, period: Duration) -> Instant {
    scheduled_start + period
}

#[cfg(test)]
mod tests {
    use super::next_iteration_start;
    use std::time::{Duration, Instant};

    const PERIOD: Duration = Duration::from_millis(20);

    #[test]
    fn advances_deadline_by_one_period() {
        let scheduled_start = Instant::now();

        assert_eq!(
            next_iteration_start(scheduled_start, PERIOD),
            scheduled_start + PERIOD,
        );
    }

    #[test]
    fn overrun_does_not_rebase_the_deadline() {
        let scheduled_start = Instant::now();
        let completed_at = scheduled_start + Duration::from_millis(24);

        let next_start = next_iteration_start(scheduled_start, PERIOD);
        assert_eq!(next_start, scheduled_start + PERIOD);
        assert!(next_start < completed_at);
        assert_eq!(
            next_iteration_start(next_start, PERIOD),
            scheduled_start + PERIOD + PERIOD,
        );
    }

    #[test]
    fn exact_deadline_does_not_add_a_delay() {
        let scheduled_start = Instant::now();
        let deadline = scheduled_start + PERIOD;

        assert_eq!(
            next_iteration_start(scheduled_start, PERIOD),
            deadline,
        );
    }

    #[test]
    fn deadlines_keep_advancing_from_the_schedule_after_long_overrun() {
        let scheduled_start = Instant::now();
        let completed_at = scheduled_start + Duration::from_millis(65);
        let next_start = next_iteration_start(scheduled_start, PERIOD);

        assert_eq!(next_start, scheduled_start + PERIOD);
        assert!(next_start < completed_at);
        assert_eq!(
            next_iteration_start(next_start, PERIOD),
            scheduled_start + PERIOD + PERIOD,
        );
    }
}
