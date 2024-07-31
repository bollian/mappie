use std::ffi::OsString;
use std::io::ErrorKind;

use adafruit_motorkit::dc::DcMotor;
use eyre::bail;
use futures_lite::prelude::*;
use linux_embedded_hal as hal;
use clap::Parser;
use eyre::{Result, WrapErr};
use pwm_pca9685::Pca9685;

const MAX_CTRL_MSG_SIZE: usize = 4096;

#[derive(Parser, Clone, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// UDP socket address to listen for control message at
    #[arg(short = 'l', long, default_value = "0.0.0.0:9090")]
    listen_addr: String,

    /// Path to the I2C bus with an attached PWM controller
    #[arg(short = 'd', long, default_value = "/dev/i2c-1")]
    i2c_dev: OsString,

    /// Address of the PCA9685 PWM controller on the I2C bus
    #[arg(short = 'a', long, default_value_t = 0x60)]
    pwm_addr: u8,
}

struct Hardware {
    motor_controller: Pca9685<hal::I2cdev>,
    fl_drive_motor: DcMotor,
    fr_drive_motor: DcMotor,
    bl_drive_motor: DcMotor,
    br_drive_motor: DcMotor,
}

fn main() -> Result<()> {
    env_logger::init();

    let args = Args::parse();

    let dev = hal::I2cdev::new(args.i2c_dev.as_os_str())
        .wrap_err("Failed to open I2C device")?;
    let mut pwm = Pca9685::new(dev, args.pwm_addr)
        .map_err(|e| eyre::eyre!("Failed to construct pwm device: {:?}", e))?;

    let fl_drive_motor = DcMotor::try_new(&mut pwm, ports::FL_DRIVE_MOTOR)
        .wrap_err("unable to construct front left motor")?;
    let fr_drive_motor = DcMotor::try_new(&mut pwm, ports::FR_DRIVE_MOTOR)
        .wrap_err("unable to construct front right motor")?;
    let bl_drive_motor = DcMotor::try_new(&mut pwm, ports::BL_DRIVE_MOTOR)
        .wrap_err("unable to construct back left motor")?;
    let br_drive_motor = DcMotor::try_new(&mut pwm, ports::BR_DRIVE_MOTOR)
        .wrap_err("unable to construct back right motor")?;

    let hardware = Hardware {
        motor_controller: pwm,
        fl_drive_motor,
        fr_drive_motor,
        bl_drive_motor,
        br_drive_motor,
    };

    let (control_tx, control_rx) = smol::channel::bounded(10);

    let controller_task = smol::spawn(async move {
        loop {
            match establish_controllers(args.clone(), control_tx.clone()).await {
                Err(e) => {
                    log::warn!("Control listener crashed: {e}");
                    log::trace!("Restarting control listener");
                },
                res => return res,
            }
        }
    });

    smol::block_on(async move {
        main_loop(hardware, control_rx)
            .race(controller_task)
            .await
    })
}

async fn establish_controllers(args: Args, control_tx: smol::channel::Sender<messages::Move>) -> Result<()> {
    let control_socket = smol::net::TcpListener::bind(args.listen_addr.as_str()).await?;

    loop {
        let (conn, _addr) = control_socket.accept().await?;
        log::info!("Accepted controller connection");
        smol::spawn(control_connection(conn, control_tx.clone())).detach();
    }
}

async fn control_connection(mut conn: smol::net::TcpStream, control_tx: smol::channel::Sender<messages::Move>) -> Result<()> {
    let mut buf = [0u8; MAX_CTRL_MSG_SIZE];

    loop {
        let len = match conn.read(&mut buf).await {
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

        let msg: messages::Move = match serde_json::from_slice(&buf[..len]) {
            Ok(m) => m,
            Err(e) => {
                log::warn!("Received invalid control message: {}", e);
                continue
            }
        };

        if control_tx.send(msg).await.is_err() {
            log::trace!("Control channel closed. Exiting control connection");
            return Ok(())
        }
    }
}

async fn main_loop(mut hardware: Hardware, control: smol::channel::Receiver<messages::Move>) -> Result<()> {
    // update at least 60 times a second
    let minimum_update_period = std::time::Duration::from_millis(16);
    let mut update_timer = smol::Timer::after(minimum_update_period);

    let mut move_x = 0.0;
    let mut move_y = 0.0;
    let mut move_rot = 0.0;

    log::trace!("Starting main loop");
    loop {
        log::debug!("Waiting for control...");
        let control = async { Some(control.recv().await) }
            .race(async {
                update_timer.next().await;
                None
            }).await;

        log::debug!("Beginning main loop iteration");
        if let Some(control) = control {
            let control = match control {
                Ok(c) => c,
                Err(_) => bail!("Control unexpectedly lost!")
            };

            move_x = control.translate.0;
            move_y = control.translate.1;
            move_rot = control.rotate;
        } else {
            log::debug!("Control timed out in main loop");
            move_x = 0.0;
            move_y = 0.0;
            move_rot = 0.0;
        }
        update_timer.set_after(minimum_update_period);

        let _ = hardware.fl_drive_motor.set_throttle(&mut hardware.motor_controller, 1.0);
        let _ = hardware.fr_drive_motor.set_throttle(&mut hardware.motor_controller, 1.0);
        let _ = hardware.bl_drive_motor.set_throttle(&mut hardware.motor_controller, 1.0);
        let _ = hardware.br_drive_motor.set_throttle(&mut hardware.motor_controller, 1.0);
        // let _ = hardware.fl_drive_motor.set_throttle(&mut hardware.motor_controller, move_y);
        // let _ = hardware.fr_drive_motor.set_throttle(&mut hardware.motor_controller, move_y);
        // let _ = hardware.bl_drive_motor.set_throttle(&mut hardware.motor_controller, move_y);
        // let _ = hardware.br_drive_motor.set_throttle(&mut hardware.motor_controller, move_y);
    }
}

mod ports {
    pub const FL_DRIVE_MOTOR: adafruit_motorkit::Motor = adafruit_motorkit::Motor::Motor1;
    pub const FR_DRIVE_MOTOR: adafruit_motorkit::Motor = adafruit_motorkit::Motor::Motor2;
    pub const BL_DRIVE_MOTOR: adafruit_motorkit::Motor = adafruit_motorkit::Motor::Motor3;
    pub const BR_DRIVE_MOTOR: adafruit_motorkit::Motor = adafruit_motorkit::Motor::Motor4;
}
