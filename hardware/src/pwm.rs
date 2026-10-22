use crate::{HardwareError, Result, UdevWaiter};

use std::io::{self, ErrorKind};
use std::ops::ControlFlow;
use std::time::Duration;

use sysfs_pwm::{Pwm as Channel, PwmChip};
use tokio::runtime::LocalRuntime;

pub struct Pwm {
    pub chip: PwmChip,
    pub pwm: Channel,
    pub channel_number: u32,
}

impl Pwm {
    pub fn new(chip: u32, channel: u32, period_ns: u32) -> Result<Self> {
        let mut udev = UdevWaiter::new()?;
        let chip = PwmChip::new(chip)?;
        chip.export(channel)?;
        let pwm = Channel::new(chip.number, channel)?;
        let settled = LocalRuntime::new().unwrap().block_on(udev.settle(Duration::from_millis(50), || {
            match pwm.set_period_ns(period_ns) {
                // happy path: we're able to use the device
                Ok(_) => ControlFlow::Break(Ok(())),
                Err(sysfs_pwm::Error::Io(e)) => match e.kind() {
                    // we still don't have permission to use the device. keep waiting for the udev
                    // reconfiguration
                    ErrorKind::PermissionDenied => ControlFlow::Continue(()),
                    // some other error has occurred that will not be solved by waiting. break
                    // immediately and bubble the error up
                    _ => ControlFlow::Break(Err(sysfs_pwm::Error::Io(e))),
                },
                // this shouldn't be possible since we're not reading from any sysfs files as part of
                // this, but if it does happen, break immediately and bubble the error up
                Err(e) => ControlFlow::Break(Err(e)),
            }
        }));

        match settled {
            Err(_) => Err(HardwareError::Io(
                io::Error::new(ErrorKind::PermissionDenied, "Timed out waiting for udev reconfiguration"))),
            Ok(Err(e)) => Err(HardwareError::Pwm(e)),
            Ok(Ok(())) => {
                pwm.enable(true)?;
                Ok(Self {
                    chip,
                    pwm,
                    channel_number: channel,
                })
            }
        }
    }
}

impl Drop for Pwm {
    fn drop(&mut self) {
        let _ = self.pwm.enable(false);
        let _ = self.chip.unexport(self.channel_number);
    }
}

/// Checks to see that the provided chip and channel combo exists
///
/// This will return true even if the channel hasn't been exported yet.
pub fn chip_channel_exists(chip: u32, channel: u32) -> bool {
    let chip = match PwmChip::new(chip) {
        Ok(c) => c,
        Err(_) => return false,
    };
    let count = chip.count().unwrap_or(0);
    count > channel
}
