use adafruit_motorkit::dc::DcMotor as AdafruitMotor;
use adafruit_motorkit::MotorError;
use eyre::{Result, WrapErr};
use signal_hook::SigId;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, atomic::AtomicBool, LazyLock};

type Pca9685 = pwm_pca9685::Pca9685<linux_embedded_hal::I2cdev>;
type Pwm = Rc<RefCell<Pca9685>>;

static PWM: CLock<Pwm> = LazyLock<Pwm>::new(
    || Rc::new(RefCell::new(Pca9685::))
)

static SIGNAL_FLAG: LazyLock<Arc<AtomicBool>> = LazyLock::new(
    || Arc::new(AtomicBool::new(false)));

thread_local! {
    static HARDWARE: RefCell<Option<SharedHardware>> = RefCell::new(None);
}

struct SharedHardware {
    pwm: Pca9685,
}

impl SharedHardware {
    fn new() {

    }
}

pub fn initialize() {

}

pub fn poll() {

}

pub struct DcMotor {
    cleanup_signal: SigId,
    motor: AdafruitMotor,
}

impl Drop for DcMotor {
    fn drop(&mut self) {}
}

impl DcMotor {
    pub fn try_new(pwm: &mut Pca9685, port: adafruit_motorkit::Motor)
        -> Result<Self>
    {
        let motor = AdafruitMotor::try_new(pwm, port)
            .wrap_err_with(|| format!("Failed to initialize motor {:?}", port))?;
        let cleanup_signal = signal_hook::flag::register(
            signal_hook::consts::SIGINT, Arc::clone(&SIGNAL_FLAG))
            .wrap_err_with(|| format!("Failed to register cleanup handler for motor {:?}", port))?;

        Ok(Self { cleanup_signal, motor })
    }

    pub fn set_throttle(&mut self, pwm: &mut Pca9685, throttle: f32) -> Result<(), MotorError> {
        self.motor.set_throttle(pwm, throttle)
    }
}
