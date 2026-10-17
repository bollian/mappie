mod config;
mod drive;

use std::path::PathBuf;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

use clap::Parser;
use eyre::{eyre, Result, WrapErr};
use hardware::HardwareError;
use hardware::lidar::{Lidar, RposError};
use futures_lite::prelude::*;
use messages::{listen_for_operator, ControlMessage, StatusMessage, SensorReading, StatusSender, ControlReceiver};

const DEFAULT_IPV4_ADDR: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 9090);
const DEFAULT_IPV6_ADDR: SocketAddr = SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 9090);

#[derive(Parser, Clone, Debug)]
#[command(author, version, about, long_about = None)]
struct Conf {
    /// UDP socket address to listen for control message at
    #[arg(short = 'l', long = "listen-addresses", default_values_t = [DEFAULT_IPV4_ADDR])]
    listen_addr: Vec<SocketAddr>,

    /// The maximum amount of time between main loop iterations
    #[arg(short = 'p', long, default_value_t = 16)]
    loop_period_ms: u64,

    /// The value below which movement commands are ignored
    #[arg(long, default_value_t = 0.1)]
    deadzone: f32,

    /// Width of the wheel base in cm (wheel ground contact to wheel ground contact)
    #[arg(long, default_value_t = 11.0 + 0.7 + 1.5)]
    wheelbase_width: f32,

    /// Length of the wheel base in cm (wheel ground contact to wheel ground contact)
    #[arg(long, default_value_t = 11.0)]
    wheelbase_length: f32,

    /// Path to the I2C bus device to find the motor controller board on
    #[arg(short = 'i', long = "i2c", default_value = "/dev/i2c-1")]
    i2c_dev_path: PathBuf,

    /// Path to the LIDAR device
    #[arg(short = 'L', long = "lidar", default_value = "/dev/ttyUSB0")]
    lidar_dev_path: String, // has to be a string because of limitations of the serialport library

    /// Path to the JSON file with the hardware configuration for this robot
    #[arg(short = 'c', long = "config", default_value = config::DEFAULT_CONFIG_PATH)]
    config_path: PathBuf,

    /// How long to wait for a message before considering a controller disconnected
    #[arg(short = 'T', long, default_value_t = 5.0)]
    timeout: f32,
}

fn main() -> Result<()> {
    env_logger::init();
    let args = Conf::parse();

    // We need to handle signals to make sure proper clean up happens, e.g. stopping motors. Given
    // the safety-critical nature of this, it happens before all hardware initialization.
    let mut sigs = async_signal::Signals::new([
        async_signal::Signal::Term,
        async_signal::Signal::Quit,
        async_signal::Signal::Int,
    ]).wrap_err("Failed to create signal handler")?;

    let rt = tokio::runtime::LocalRuntime::new()
        .wrap_err("Failed to create async runtime")?;
    return rt.block_on(async {
        connection_loop(args)
            .race(async {
                match sigs.next().await {
                    Some(Ok(sig)) => {
                        Err(eyre!("Exited via signal {:?}", sig))
                    }
                    Some(Err(io_err)) => {
                        Err(eyre!("Unexpected error while waiting for signal: {}", io_err))
                    }
                    None => {
                        // the signals stream should only end when it's impossible for us to
                        // receive signals any more, which will only be the case after the process
                        // exits
                        unreachable!()
                    }
                }
            }).await
    })
}

async fn connection_loop(
    args: Conf,
) -> Result<()> {
    loop {
        log::info!("Listening for new controller connection...");
        let (status, controller) = listen_for_operator(args.listen_addr.iter().copied()).await?;
        log::info!("Controller connected");
        match main_loop(args.clone(), status, controller).await {
            Ok(_) => {
                log::info!("Clean shutdown");
                return Ok(());
            }
            Err(e) => {
                log::warn!("Error during main loop: {e:?}. Restarting.")
            }
        }
        // if let Err(e) = drive.stop() {
        //     return Err(e)
        //         .wrap_err("Failed to stop drive motors during control reset");
        // }
    }
}

async fn main_loop(
    args: Conf,
    status: StatusSender,
    controller: ControlReceiver,
) -> Result<()> {
    // follow the configured minimum update rate, even when a message isn't received
    let control_task = tokio::spawn(controller_task(args.clone(), controller));
    let report_task = tokio::spawn(reporter_task(args.clone(), status));
    let result: Result<(Result<()>, Result<()>), _> = tokio::try_join!(
        control_task, report_task
    );
    match result {
        Ok((contros_res, report_res)) => {
            let mut final_res = Ok(());
            if let Err(e) = contros_res {
                final_res = Err(e.wrap_err("Control task exited with error"));
            }
            if let Err(e) = report_res {
                final_res = Err(e.wrap_err("Report task exited with error"));
            }
            return final_res;
        }
        Err(e) => {
            log::error!("Failed to join control or report tasks: {e:?}");
            return Err(e.into())
        }
    }
}

async fn controller_task(args: Conf, controller: ControlReceiver) -> Result<()> {
    let mut drive = drive::Drive::new(args.clone())
        .wrap_err("Failed to initialize drive system")?;

    log::debug!("Starting main loop");
    let timeout_duration = Duration::from_secs_f32(args.timeout);
    let mut ctrl_msg;

    loop {
        log::trace!("Waiting for control...");
        match tokio::time::timeout(timeout_duration, controller.receive()).await {
            Ok(Ok(c)) => ctrl_msg = c,
            Ok(Err(e)) => {
                let msg = "Control unexpectedly lost!";
                log::info!("{msg}");
                return Err(e)
                    .wrap_err(msg)
            }
            Err(timeout) => {
                let msg = "Timed out waiting for control message in main loop";
                log::info!("{msg}: {timeout}");
                return Err(timeout)
                    .wrap_err(msg)
            }
        }

        match ctrl_msg {
            ControlMessage::Move(m) => {
                log::trace!("Driving with {m}");
                drive.control(m)
            },
            ControlMessage::EstablishControl => {
                log::debug!("Unexpected handshake message from controller. Ignoring");
            }
        }
    }
}

async fn reporter_task(args: Conf, mut status: StatusSender) -> Result<()> {
    let mut lidar = Lidar::new(args.lidar_dev_path.as_str())
        .wrap_err("Failed to open LIDAR device")?;

    log::debug!("Starting sensors loop");
    let minimum_update_period = Duration::from_millis(args.loop_period_ms);
    let mut interval = tokio::time::interval(minimum_update_period);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        log::trace!("Waiting for sensor data...");
        interval.tick().await;
        let scan = match lidar.grab_scan() {
            Ok(s) => s,
            Err(HardwareError::Lidar(RposError::OperationTimeout)) => {
                log::warn!("Timeout grabbing LIDAR scan... ignoring");
                continue
            },
            Err(e) => return Err(e.into())
        };
        let reading = SensorReading { scan };
        status.update(&StatusMessage::SensorReading(reading)).await?;
    }
}
