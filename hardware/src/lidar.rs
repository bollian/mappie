use std::time::Duration;
use rplidar_drv::{RplidarDevice, ScanMode};
pub use rplidar_drv::{ScanPoint, RposError};
use serde::{Serialize, Deserialize};
use serialport::SerialPort;

use crate::{HardwareError, Result};

const RPLIDAR_BAUD_RATE: u32 = 115200;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Scan {
    pub points: Vec<ScanPoint>,
    pub discarded_count: u32,
}

pub struct Lidar {
    dev: RplidarDevice<dyn SerialPort>,
}

impl Lidar {
    pub fn new(dev_path: &str) -> Result<Self> {
        let port = serialport::new(dev_path, RPLIDAR_BAUD_RATE)
            .data_bits(serialport::DataBits::Eight)
            .flow_control(serialport::FlowControl::None)
            .parity(serialport::Parity::None)
            .stop_bits(serialport::StopBits::One)
            .timeout(Duration::from_millis(5))
            .dtr_on_open(false)
            .open()
            .inspect_err(|e| log::error!("Failed to open serial port to LIDAR sensor at '{dev_path}': {e}"))?;

        let channel = rpos_drv::Channel::new(rplidar_drv::RplidarHostProtocol::new(), port);
        let mut rplidar = rplidar_drv::RplidarDevice::new(channel);
        let device_info = rplidar.get_device_info()
            .inspect_err(|e| log::error!("LIDAR error when getting device info: {e}"))?;

        struct HexBytes<'a>(&'a [u8]);

        impl<'a> std::fmt::LowerHex for HexBytes<'a> {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                for byte in self.0 {
                    f.write_fmt(format_args!("{:02x}", byte))?;
                }
                Ok(())
            }
        }

        let model = device_info.model;
        let major = device_info.firmware_version >> 8;
        let minor = device_info.firmware_version & 0xff;
        let hardware = device_info.hardware_version;
        let serial_number = HexBytes(device_info.serialnum.as_slice());
        log::info!(
"Connected to LIDAR:
Model: {model}
Firmware Version: {major}.{minor}
Hardware Version: {hardware}
Serial Number: {serial_number:02x}");

        let device_health = rplidar.get_device_health()
            .inspect_err(|e| log::error!("LIDAR error when getting device health: {e}"))?;
        match device_health {
            rplidar_drv::Health::Healthy => log::info!("LIDAR device is healthy"),
            rplidar_drv::Health::Warning(code) => {
                return Err(HardwareError::LidarHealthWarning(code))
            }
            rplidar_drv::Health::Error(code) => {
                return Err(HardwareError::LidarHealthError(code))
            }
        }

        let typical_scan_mode = rplidar.get_typical_scan_mode()
            .inspect_err(|e| log::error!("Failed to get typical scan mode from LIDAR: {e}"))?;
        log::info!("LIDAR using typical scan mode {typical_scan_mode}");

        match rplidar.check_motor_ctrl_support() {
            Ok(support) => if support == true {
                log::info!("Accessory board is detected and support motor control, starting motor...");
                rplidar.set_motor_pwm(600)
                    .inspect_err(|e| log::error!("Failed to start LIDAR motor: {e}"))?;
            } else {
                log::warn!("Accessory board is detected, but doesn't support motor control");
            },
            Err(e) => {
                log::warn!("Accessory board isn't detected: {e}");
            }
        }

        let ScanMode { id, name, max_distance, ans_type: _, us_per_sample } = rplidar.start_scan()?;
        log::info!("LIDAR scanning using {name}({id}) mode: {max_distance}m max distance, {us_per_sample}µs per sample");

        return Ok(Self { dev: rplidar })
    }

    pub fn grab_scan(&mut self) -> Result<Scan> {
        let scan = self.dev.grab_scan()?;
        let orig_len = scan.len();
        let points: Vec<_> = scan.into_iter()
            .filter(|p| p.is_valid() && p.meters() > f32::EPSILON)
            .collect();
        let discarded = orig_len - points.len();
        Ok(Scan {
            discarded_count: discarded as u32,
            points,
        })
    }
}
