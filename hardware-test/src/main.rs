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
    let poll_timeout = Timespec {
        tv_sec: period.as_secs() as i64,
        tv_nsec: i64::from(period.subsec_nanos()),
    };
    let mut stdin = std::io::stdin();
    let mut stdin_buf = [0; 256];
    let mut previous_periodic = Instant::now();
    loop {
        if args.no_prompt {
            std::thread::sleep(period);
        } else {
            let poll_count = poll(&mut [PollFd::new(&stdin, PollFlags::IN)], Some(&poll_timeout))
                .context("Stdin poll operation failed")?;
            if poll_count == 1 {
                // check for if the user pressed Enter
                let count = stdin.read(&mut stdin_buf[..])?;
                if stdin_buf[..count].contains(&b'\n') {
                    break
                }
                // clear the buffer because we don't care about anything aside from newlines
                stdin_buf[..count].fill(0);
            }
        }

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
    }

    anyhow::Result::<()>::Ok(())
}
