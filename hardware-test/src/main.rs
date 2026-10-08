mod i2c_motors;
mod lidar;
mod motors;
mod pwm;

use std::io::Read;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use hardware::gpio::Gpio;
use rustix::event::{PollFd, PollFlags, poll};
use rustix::time::Timespec;

use i2c_motors::*;
use lidar::*;
use motors::*;
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
}

#[derive(Subcommand)]
enum Command {
    Motors(MotorsArgs),
    Lidar(LidarArgs),
    Pwm(PwmArgs),
    I2cMotors(I2cMotorsArgs),
}

fn main() -> Result<()> {
    env_logger::init();

    let args = Args::parse();
    let mut motors = None;
    let mut lidar = None;
    let mut pwm = None;
    let mut i2c_motors = None;
    match args.subcommand {
        Command::Motors(args) => {
            motors = Some(MotorsTest::new(args)
                .context("Failed to initialize motors test")?);
        }
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

    let _gpio = if args.skip_gpio {
        None
    } else {
        Some(Gpio::new().context("Failed to initialize GPIO")?)
    };

    if let Some(motors) = &mut motors {
        motors.start();
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

    let mut stdin = std::io::stdin();
    let mut stdin_buf = [0; 256];
    loop {
        const PRINT_PERIOD_MS: i64 = 20;
        const PRINT_PERIOD: Timespec = Timespec {
            tv_sec: 0,
            tv_nsec: PRINT_PERIOD_MS * 1000 * 1000, // refresh 50 times a second
        };

        if args.no_prompt {
            std::thread::sleep(std::time::Duration::from_millis(PRINT_PERIOD_MS as _));
        } else {
            let poll_count = poll(&mut [PollFd::new(&stdin, PollFlags::IN)], Some(&PRINT_PERIOD))
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

        if let Some(pwm_test) = &mut pwm {
            pwm_test.periodic()?;
        }
        if let Some(lidar_test) = &mut lidar {
            lidar_test.periodic()?;
        }
    }

    anyhow::Result::<()>::Ok(())
}
