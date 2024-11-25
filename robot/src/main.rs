// mod hardware;
mod motor_hat;
mod drive;

use std::ffi::OsString;
use std::io::ErrorKind;

use clap::Parser;
use eyre::{eyre, bail, Result, WrapErr};
use futures_lite::prelude::*;
use linux_embedded_hal as hal;
use smol::net::UdpSocket;

pub type Pwm = pwm_pca9685::Pca9685<hal::I2cdev>;

#[derive(Parser, Clone, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// UDP socket address to listen for control message at
    #[arg(short = 'l', long, default_value = "0.0.0.0:9090")]
    listen_addr: String,

    /// Path to the I2C bus device file with an attached PWM controller
    #[arg(short = 'd', long, default_value = "/dev/i2c-1")]
    i2c_dev: OsString,

    /// Address of the PCA9685 PWM controller on the I2C bus
    #[arg(short = 'a', long, default_value_t = 0x60)]
    pwm_addr: u8,

    /// The maximum amount of time between main loop iterations
    #[arg(short = 'p', long, default_value_t = 16)]
    loop_period_ms: u64,

    /// The value below which movement commands are ignored
    #[arg(long, default_value_t = 0.2)]
    deadzone: f32
}

fn main() -> Result<()> {
    env_logger::init();
    let args = Args::parse();

    // We need to handle signals to make sure proper clean up happens, e.g. stopping motors. Given
    // the safety-critical nature of this, it happens before all hardware initialization.
    let mut sigs = async_signal::Signals::new([
        async_signal::Signal::Term,
        async_signal::Signal::Quit,
        async_signal::Signal::Int,
    ]).wrap_err("Failed to create signal handler")?;

    let dev = hal::I2cdev::new(args.i2c_dev.as_os_str())
        .wrap_err("Failed to open I2C device")?;
    let mut pwm = adafruit_motorkit::init_pwm(Some(dev))
        .wrap_err("Failed to initialize PWM controller on motor hat")?;

    let drive = drive::Drive::new(args.clone(), &mut pwm)
        .wrap_err("Failed to initialize drive system")?;

    let mut hardware = scopeguard::guard((pwm, drive), |(mut pwm, mut drive)| {
        // reset all hardware to an off state when main() exits
        drive.reset(&mut pwm);
    });
    let (pwm, drive) = &mut *hardware;

    // create the task to listen for new, connecting operators
    let (control_tx, control_rx) = smol::channel::bounded(10);
    let _listen_task = smol::spawn({
        let args = args.clone();
        async move {
            match establish_operators(args.clone(), control_tx.clone()).await {
                Err(e) => {
                    log::warn!("Control listener crashed: {e}");
                    panic!("Control listener crashed: {e}")
                    // log::trace!("Restarting control listener");
                },
                res => res,
            }
        }
    });

    let res = smol::block_on(async {
        main_loop(args, pwm, drive, control_rx)
            // The listen task is critical. If it exits unnexpectedly, exit the code
            // .race(listen_task)
            // Exit cleanly after shutdown signal
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
            })
            .await
    });

    res
}

/// Accepts connections from new operators
async fn establish_operators(args: Args, control_tx: smol::channel::Sender<messages::Move>) -> Result<()> {
    // let control_socket = smol::net::TcpListener::bind(args.listen_addr.as_str()).await?;

    // loop {
        // log::info!("Waiting for operator...");
        // let (conn, _addr) = control_socket.accept().await?;
        // log::info!("Accepted operator connection");
    log::info!("Using connectionless operators...");
    let uconn = smol::net::UdpSocket::bind(args.listen_addr.as_str()).await?;
    smol::spawn(control_connection(uconn, control_tx.clone())).detach();
    Ok(())
    // }
}

/// Decodes messages from operators and sends them to the main loop
async fn control_connection(conn: UdpSocket, control_tx: smol::channel::Sender<messages::Move>) -> Result<()> {
    let mut buf = [0u8; messages::Move::POSTCARD_COBS_BUFFER_MAX_SIZE];
    // let mut buf_insert_index = 0;
    let mut received_msg_count = 0;

    loop {
        let len = match conn.recv(&mut buf).await {
            Ok(len) => len,
            Err(e) => match e.kind() {
                ErrorKind::UnexpectedEof => {
                    eyre::bail!("Unexpected EOF while receiving from operator")
                },
                _ => {
                    return Err(eyre::Report::new(e)
                        .wrap_err("Unhandled I/O error while receiving from operator"))
                }
            }
        };

        if len == 0 {
            log::warn!("Connection to operator closed!");
            return Ok(())
        }
        if buf[len - 1] != 0 {
            log::warn!("Didn't receive complete datagram! ({} msgs received)", received_msg_count);
        }

        // we've reached the end of a cobs packet, read this in as a message
        let msg: messages::Move = match postcard::from_bytes_cobs(&mut buf[..len]) {
            Ok(msg) => msg,
            Err(e) => {
                log::warn!("Received invalid control message: {} ({} msgs received)", e, received_msg_count);
                continue
            }
        };
        // next_msg_start = i + 1;
        received_msg_count += 1;

        if control_tx.send(msg).await.is_err() {
            log::info!("Control channel closed. Exiting control connection");
            return Ok(())
        }

        // let last_insert_index = buf_insert_index;
        // buf_insert_index += len;
        // let mut next_msg_start = 0;

        // Loop through the buffer, finding any full received messages by looking for the
        // terminator byte (value 0)
        // for i in last_insert_index..buf_insert_index {
        //     let byte = buf[i];
        //     if byte == 0 {
        //     }
        // }

        // move any unprocessed data to the front of the buffer and reset the buf_insert_index,
        // effectively erasing all the messages we just deserialized and processed
        // buf.copy_within(next_msg_start..buf_insert_index, 0);
        // buf_insert_index -= next_msg_start;
    }
}

async fn main_loop(
    args: Args,
    pwm: &mut Pwm,
    drive: &mut drive::Drive,
    control: smol::channel::Receiver<messages::Move>
) -> Result<()> {
    // follow the configured minimum update rate, even when a message isn't received
    let minimum_update_period = std::time::Duration::from_millis(args.loop_period_ms);
    let mut update_timer = smol::Timer::after(minimum_update_period);

    let mut movement;

    log::debug!("Starting main loop");
    loop {
        log::trace!("Waiting for control...");
        let control = async { Some(control.recv().await) }
            .race(async {
                update_timer.next().await;
                None
            }).await;

        log::trace!("Beginning main loop iteration");
        if let Some(control) = control {
            movement = match control {
                Ok(c) => c,
                Err(_) => bail!("Control unexpectedly lost!")
            };
        } else {
            log::trace!("Control timed out in main loop");
            movement = messages::Move::stop();
        }
        update_timer.set_after(minimum_update_period);

        drive.main_loop(pwm, movement);
    }
}
