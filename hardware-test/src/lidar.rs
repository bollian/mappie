use anyhow::Result;
use clap::Parser;
use hardware::HardwareError;
use hardware::lidar::{Lidar, RposError};

#[derive(Parser, Clone, Debug)]
pub struct LidarArgs {
    /// Path to the serial port of the LiDAR sensor
    #[arg(short = 'p', long, default_value = "/dev/ttyUSB0")]
    serial_port: String,

    /// Baudrate of the serial connection to the LiDAR sensor
    #[arg(short = 'b', long, default_value_t = 115200)]
    baudrate: u32,

    /// PWM value to control the motor speed
    #[arg(short = 'm', long,
        value_parser = clap::value_parser!(u16).range(0..1023),
        default_value_t = 600)]
    motor_pwm: u16,
}

pub struct LidarTest {
    _args: LidarArgs,
    lidar: Lidar,
}

impl LidarTest {
    pub fn new(args: LidarArgs) -> Result<Self> {
        let lidar = Lidar::new(args.serial_port.as_str())?;
        Ok(Self {
            _args: args,
            lidar,
        })
    }

    pub fn periodic(&mut self) -> Result<()> {
        let scan = match self.lidar.grab_scan() {
            Ok(s) => s,
            Err(HardwareError::Lidar(RposError::OperationTimeout)) => {
                log::warn!("Timeout grabbing LIDAR scan... ignoring");
                return Ok(())
            }
            Err(e) => {
                anyhow::bail!("Failed to grab LIDAR scan: {e}")
            },
        };

        for point in scan.points {
            print!("{}∠{}, ", point.dist_mm_q2, point.angle_z_q14);
        }
        println!();

        Ok(())
    }
}
