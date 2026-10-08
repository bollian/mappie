pub mod hiwonder_motor_board;
#[cfg(target_arch = "aarch64")]
pub mod l298n_motor;
pub mod lidar;
pub mod pwm;

use std::io;
use std::mem::MaybeUninit;
use std::ops::ControlFlow;
use std::os::fd::BorrowedFd;
use std::time::Duration;

use rustix::fd::{OwnedFd, AsFd};
use rustix::fs::inotify::{self, WatchFlags};
use thiserror::Error;
use tokio::io::Interest;
use tokio::io::unix::AsyncFd;
use tokio::time::Instant;

#[cfg(target_arch = "aarch64")]
pub use l298n_motor as motor;
#[cfg(target_arch = "aarch64")]
pub use rppal::gpio as gpio;

pub type Result<T> = std::result::Result<T, HardwareError>;

#[derive(Error, Debug)]
pub enum HardwareError {
    #[cfg(target_arch = "aarch64")]
    #[error("GPIO error: {0}")]
    Gpio(#[from] rppal::gpio::Error),
    #[error("PWM error: {0}")]
    Pwm(#[from] sysfs_pwm::Error),
    #[error("OS error: {0}")]
    Io(#[from] io::Error),
    #[error("Serial port error: {0}")]
    Serialport(#[from] serialport::Error),
    #[error("LIDAR error: {0}")]
    Lidar(#[from] rpos_drv::RposError),
    #[error("LIDAR self-reported health warning code {0}")]
    LidarHealthWarning(u16),
    #[error("LIDAR self-reported health error code {0}")]
    LidarHealthError(u16),
}

struct AsyncInotify {
    inotify: AsyncFd<OwnedFd>,
    buf: Box<[MaybeUninit<u8>; 4096]>,
}

impl AsyncInotify {
    fn new() -> io::Result<Self> {
        let inotify_fd = inotify::init(inotify::CreateFlags::NONBLOCK)?;
        Ok(Self {
            inotify: AsyncFd::with_interest(inotify_fd, Interest::READABLE)?,
            buf: Box::new([MaybeUninit::uninit(); _]),
        })
    }

    fn add_watch<P: rustix::path::Arg>(&mut self, path: P, mask: WatchFlags) -> io::Result<()> {
        inotify::add_watch(self.inotify.get_ref().as_fd(), path, mask)?;
        Ok(())
    }

    fn stream(&mut self) -> InotifyStream<'_> {
        InotifyStream {
            reader: inotify::Reader::new(self.inotify.as_fd(), self.buf.as_mut_slice()),
            async_fd: &self.inotify,
        }
    }
}

struct InotifyStream<'i> {
    reader: inotify::Reader<'i, BorrowedFd<'i>>,
    async_fd: &'i AsyncFd<OwnedFd>,
}

impl<'i> InotifyStream<'i> {
    async fn next(&mut self) -> io::Result<inotify::Event<'_>> {
        let _guard = self.async_fd.readable().await?;
        Ok(self.reader.next()?)
    }
}

/// Used to wait for udev events to reconfigure hardware
///
/// When reconfiguring hardware, it is sometimes necessary for udev events to fire so that
/// permissions can be set or other automatic configuration can occur. To do this, create this
/// object before reconfiguring your hardware, then call [`Self::settle`] after reconfiguring to
/// wait for all udev events to clear.
///
/// It is safe to have multiple `UdevWaiter`s at the same time so long as you don't run out of
/// inotify contexts.
pub struct UdevWaiter {
    inotify: AsyncInotify,
}

impl UdevWaiter {
    pub fn new() -> io::Result<Self> {
        let mut inotify = AsyncInotify::new()?;
        // we're watching for the /run/udev/queue file to get deleted
        inotify.add_watch("/run/udev", WatchFlags::DELETE)?;
        Ok(Self {
            inotify: inotify,
        })
    }

    /// Asynchronously wait for the udev event queue to clear
    ///
    /// Since multiple hardware events might be occurring at the same time on the system, you must
    /// provide the `retry` function to check and see if the desired hardware state has been met.
    /// This will be called before checking for the queue, each time the queue is cleared afterwards,
    /// and once at the end if we time out. Once `retry` returns [`ControlFlow::Break`],
    /// [`Self::settle`] will return the contained value result with `Ok`.
    ///
    /// If `retry` only ever returns [`ControlFlow::Continue`], this function returns `Err` after the
    /// `timeout` has transpired.
    pub async fn settle<T, R>(&mut self, timeout: Duration, mut retry: R) -> io::Result<T>
    where
        R: FnMut() -> ControlFlow<T>,
    {
        if let ControlFlow::Break(v) = retry() {
            return Ok(v)
        }

        let end_time = Instant::now() + timeout;
        loop {
            let r = tokio::time::timeout_at(end_time,
                async {
                    let mut stream = self.inotify.stream();
                    loop {
                        let event = stream.next().await?;
                        if event.file_name() == Some(c"queue") {
                            return Ok(())
                        }
                        // if we got here, we're going to loop and try reading
                        // events again since there might be multiple events
                    }
                }
            );

            match r.await {
                Ok(Ok(())) => if let ControlFlow::Break(v) = retry() {
                    // our desired end condition was met
                    return Ok(v)
                } /* else retry() == Continue and we will loop and retry */,
                Ok(Err(e)) => return Err(e),
                Err(e) => match retry() { // we timed out
                    // the queue never cleared (maybe the machine is under heavy load or there are
                    // a lot of events), however it appears that the reconfiguration we care about
                    // was successful
                    ControlFlow::Break(v) => return Ok(v),
                    // our hardware was never reconfigured
                    _ => return Err(io::Error::new(io::ErrorKind::TimedOut, e)),
                }
            }
        }
    }
}
