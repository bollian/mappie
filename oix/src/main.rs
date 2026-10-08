mod banner;
mod lidar_view;

use std::fmt::{Display, Formatter};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Result, bail};
use gilrs::{Axis, GamepadId};
use hardware::lidar::{Lidar, Scan};
use messages::{ControlSender, StatusReceiver, ControlMessage, Move, SensorReading, StatusMessage};
use tokio::sync::Notify;
use tokio::time::{sleep, timeout};
use xilem::{AnyWidgetView, Color, WidgetView, EventLoop, Xilem, WindowOptions};
use xilem::core::{MessageProxy, fork, map_state};
use xilem::dpi::LogicalSize;
use xilem::masonry::layout::Length;
use xilem::style::Style;
use xilem::view::*;

use banner::Banner;
use lidar_view::LidarView;

const CONN_DISCONNECT_TIMEOUT: Duration = Duration::from_secs(5);

struct AppData {
    gilrs_context: Arc<Mutex<gilrs::Gilrs>>,
    selected_gamepad: Option<GamepadId>,
    known_gamepads: Vec<(GamepadId, String)>,

    banner: Option<Banner>,

    /// IP or DNS address of the robot
    address: String,

    /// Port for the robot's server
    port: Option<u16>,

    /// File path to the LIDAR device
    lidar_dev_path: String,

    state: AppState,
}

enum AppState {
    Setup,
    Connecting,
    Driving(LidarView),
    LocalLidar(LidarView),
}

impl Display for AppState {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", match self {
            AppState::Setup => "Setup",
            AppState::Connecting => "Connecting",
            AppState::Driving(_) => "Driving",
            AppState::LocalLidar(_) => "LocalLidar",
        })
    }
}

impl AppData {
    fn new() -> Result<Self> {
        let gilrs_context = match gilrs::Gilrs::new() {
            Ok(g) => g,
            Err(e) => bail!("Unable to initialize gamepad subsystem: {e}")
        };
        let gamepads = gilrs_context.gamepads()
            .map(|(id, gamepad)| (id, gamepad.name().to_string()))
            .collect();

        Ok(Self {
            gilrs_context: Arc::new(Mutex::new(gilrs_context)),
            selected_gamepad: None,
            known_gamepads: gamepads,
            banner: None,
            address: "mappie".to_string(),
            port: Some(9090),
            lidar_dev_path: "/dev/ttyUSB0".to_string(),
            state: AppState::Setup,
        })
    }

    fn display(&mut self) -> impl WidgetView<Self> + use<> {
        let control_task = if matches!(self.state, AppState::Connecting | AppState::Driving(_)) {
            Some(task(
                Self::create_control_tasks,
                |app: &mut Self, sensor_reading| {
                    match sensor_reading {
                        Ok(sensor_reading) => match &mut app.state {
                            AppState::Connecting => {
                                let mut lidar_view = LidarView::new();
                                lidar_view.give_data(sensor_reading.scan);
                                app.state = AppState::Driving(lidar_view);
                            },
                            AppState::Driving(lidar_view) => {
                                lidar_view.give_data(sensor_reading.scan);
                            }
                            _ => unreachable!(),
                        },
                        Err(banner) => {
                            app.banner = Some(banner);
                            app.state = AppState::Setup;
                        },
                    }
                }
            ))
        } else { None };
        let lidar_read_task = if let AppState::LocalLidar(_) = &mut self.state {
            Some(task(
                Self::read_lidar_task,
                |app: &mut Self, lidar_scan| {
                    if let AppState::LocalLidar(lidar_view) = &mut app.state {
                        match lidar_scan {
                            Ok(scan) => lidar_view.give_data(scan),
                            Err(banner) => {
                                app.banner = Some(banner);
                                // TODO: fix the spurious timeout errors from the lidar
                                // app.state = AppState::Setup;
                            }
                        }
                    }
                }
            ))
        } else { None };

        let main_view: Box<AnyWidgetView<Self>> = match &mut self.state {
            AppState::Setup => {
                let port_str = match self.port {
                    Some(p) => format!("{}", p),
                    None => "".to_string(),
                };
                let remote_controls = flex_col((
                    virtual_scroll(0..(self.known_gamepads.len() as i64), Self::gamepad_select_view)
                        .border(Color::new([0.0, 0.0, 0.0, 1.0]), Length::px(1.0)),
                    text_input(self.address.clone(), |app: &mut Self, new_addr| app.address = new_addr),
                    text_input(port_str, |app: &mut Self, new_port| {
                        if new_port == "" {
                            app.port = None
                        } else if let Ok(port) = new_port.parse() {
                            app.port = Some(port);
                        }
                    }),
                    text_button("Connect", |app: &mut Self| app.state = AppState::Connecting)
                        .disabled(self.port.is_none() || self.address == "" || self.selected_gamepad.is_none())
                ));
                let local_controls = flex_col((
                    flex_row((
                        label("USB Device"),
                        text_input(self.lidar_dev_path.clone(), |app: &mut Self, new_path| {
                            app.lidar_dev_path = new_path;
                        }),
                    )),
                    text_button("Start Scanning", |app: &mut Self| app.state = AppState::LocalLidar(LidarView::new())),
                ));
                Box::new(flex_row((remote_controls, local_controls)))
            }
            AppState::Connecting => {
                // this has to be an anyview, otherwise the control_task will be rebuilt when we go
                // into the Driving state
                let wait_screen: Box<AnyWidgetView<Self>> = Box::new(flex_col((
                    label("Connecting..."),
                    spinner(),
                )));
                Box::new(fork(wait_screen, control_task))
            }
            AppState::Driving(lidar_view) => {
                // this has to be an anyview, otherwise the control_task will be rebuilt when we
                // come from the Connecting state
                let view: Box<AnyWidgetView<Self>> = Box::new(map_state(lidar_view.display(), |app: &mut Self| {
                    if let AppState::Driving(lidar_view) = &mut app.state {
                        lidar_view
                    } else { unreachable!() }
                }));
                Box::new(fork(view, control_task))
            }
            AppState::LocalLidar(lidar_view) => {
                let view = map_state(lidar_view.display(), |app: &mut Self| {
                    if let AppState::LocalLidar(lidar_view) = &mut app.state {
                        lidar_view
                    } else { unreachable!() }
                });
                Box::new(fork(view, lidar_read_task))
            }
        };

        flex_col((
            Banner::opt_display(&mut self.banner, |app: &mut Self| &mut app.banner),
            main_view.flex(2.0),
        ))
    }

    fn gamepad_select_view(&mut self, index: i64) -> impl WidgetView<Self> + use<> {
        let selected = if let Some(selected_gamepad) = self.selected_gamepad {
            self.known_gamepads[index as usize].0 == selected_gamepad
        } else { false };

        let row = flex_row((
            text_button("Select", move |app: &mut Self| {
                app.selected_gamepad = Some(app.known_gamepads[index as usize].0);
            }),
            label(self.known_gamepads[index as usize].1.as_str()),
        ));
        if selected {
            row.border(Color::new([0.0, 0.0, 1.0, 1.0]), Length::px(2.0)).boxed()
        } else { row.padding(Length::px(2.0)).boxed() }
    }

    fn create_control_tasks(proxy: MessageProxy<Result<SensorReading, Banner>>, app: &mut Self)
        -> impl Future<Output = ()> + use<>
    {
        log::trace!("Creating control tasks...");
        let address = app.address.clone();
        let port = app.port;
        let gilrs_context = Arc::clone(&app.gilrs_context);
        let selected_gamepad = app.selected_gamepad.expect("No gamepad was selected");

        async move {
            let Some(port) = port else {
                message(&proxy, Err(crate::error!("No port number was specified when starting to connect")), "No port banner");
                return
            };
            let robot_addr = match tokio::net::lookup_host((address.as_str(), port)).await
                .map(|mut iter| iter.next())
            {
                Ok(Some(addr)) => addr,
                Ok(None) => {
                    message(&proxy, Err(crate::error!("Unable to resolve the address '{address}'")),
                        "Sending failed DNS lookup");
                    return
                }
                Err(e) => {
                    message(&proxy, Err(crate::error!("Unable to resolve the address '{address}': {e}")),
                        "Sending failed DNS lookup");
                    return
                }
            };
            log::debug!("Starting connection to robot at {robot_addr}");
            let (control, status) = match messages::establish_control(robot_addr).await {
                Ok(c) => c,
                Err(e) => {
                    message(&proxy, Err(crate::error!("Failed to connect: {e}")), "Connection failed banner");
                    return
                }
            };

            let notify = Arc::new(Notify::new());
            let control_notify = Arc::clone(&notify);
            let read_notify = Arc::clone(&notify);
            let _notify_guard = scopeguard::guard(notify, |notify| {
                log::trace!("Dropping the create_control_tasks future");
                notify.notify_waiters();
            });

            log::trace!("Starting control and status tasks");
            let control_task = tokio::spawn(Self::control_task(control, control_notify, gilrs_context, selected_gamepad));
            let read_task = tokio::spawn(Self::robot_status_task(proxy.clone(), status, read_notify));

            let task_res = tokio::select! {
                control_res = control_task => {
                    log::info!("Robot control task exited");
                    control_res
                },
                read_res = read_task => {
                    log::info!("Robot sensor reading task exited");
                    read_res
                },
            };

            log::trace!("control task selected returned {task_res:?}");
            match task_res {
                Err(join_err) => message(&proxy, Err(crate::warn!("Background control task ended unexpectedly: {join_err}")), "Unexpected background task exit"),
                Ok(Err(banner)) => message(&proxy, Err(banner), "Forwarding task error"),
                Ok(Ok(())) => return,
            }
        }
    }

    async fn control_task(
        mut control: ControlSender,
        control_notify: Arc<Notify>,
        gilrs_context: Arc<Mutex<gilrs::Gilrs>>,
        selected_gamepad: GamepadId,
    ) -> Result<(), Banner> {
        loop {
            tokio::select! {
                _ = control_notify.notified() => {
                    log::info!("Shutdown signal received. Exiting control task");
                    return Ok(())
                },
                res = Self::send_control(&mut control, &*gilrs_context, selected_gamepad) => if let Err(banner) = res {
                    return Err(banner)
                }
            }
        }
    }

    async fn send_control(
        channel: &mut ControlSender,
        gilrs_context: &Mutex<gilrs::Gilrs>,
        selected_gamepad: GamepadId
    ) -> Result<(), Banner>
    {
        sleep(Duration::from_millis(20)).await; // send control data 50 times a second

        // to reduce input latency, wait until the underlying socket is actually
        // writable
        if let Err(e) = channel.wait_until_writable().await {
            return Err(crate::error!("Robot disconnected while waiting for writability: {e}"))
        }

        let msg = {
            let mut gilrs_context = gilrs_context.lock().unwrap();
            while let Some(_) = gilrs_context.next_event() {} // read all events to update gamepad state
            let gamepad = match gilrs_context.connected_gamepad(selected_gamepad) {
                Some(g) => g,
                None => {
                    return Err(crate::warn!("Selected gamepad was disconnected"));
                }
            };
            let x = gamepad.value(Axis::LeftStickX);
            let y = gamepad.value(Axis::LeftStickY);
            let rotate = gamepad.value(Axis::RightStickX);
            ControlMessage::Move(Move {
                translate: [x, y].into(),
                rotate
            })
        };

        if let Err(e) = channel.control(msg).await {
            return Err(crate::error!("Robot disconnected while sending movement message: {e}"))
        }
        return Ok(())
    }

    async fn robot_status_task(
        msg_proxy: MessageProxy<Result<SensorReading, Banner>>,
        mut status: StatusReceiver,
        read_notify: Arc<Notify>,
    ) -> Result<(), Banner> {
        loop {
            tokio::select! {
                _ = read_notify.notified() => {
                    log::info!("Shutdown signal received. Exiting read task");
                    return Ok(())
                }
                reading = timeout(CONN_DISCONNECT_TIMEOUT, status.receive()) => match reading {
                    Ok(Ok((reading, _addr))) => match reading {
                        StatusMessage::SensorReading(reading) => if let Err(e) = msg_proxy.message(Ok(reading)) {
                            return Err(crate::error!("Failed to send sensor data to main view: {e}"))
                        }
                    }
                    Ok(Err(e)) => {
                        return Err(crate::error!("Robot disconnected unexpectedly: {e}"))
                    }
                    Err(_) => { // timeout
                        return Err(crate::warn!("Connection to robot timed out"));
                    }
                }
            };
        }
    }

    fn read_lidar_task(proxy: MessageProxy<Result<Scan, Banner>>, app: &mut Self)
        -> Pin<Box<dyn Future<Output = ()> + Send + 'static>>
    {
        let mut lidar = match Lidar::new(app.lidar_dev_path.as_str()) {
            Ok(l) => l,
            Err(e) => {
                message(&proxy, Err(crate::error!("Failed to initialize LIDAR: {e}")),
                    "Reporting LIDAR initialization to main view");
                return Box::pin(async {})
            }
        };
        Box::pin(async move {
            loop {
                let data = lidar.grab_scan()
                    .map_err(|e| crate::error!("LIDAR seemingly disconnected: {e}"));
                if let Err(e) = proxy.message(data) {
                    log::warn!("Failed to report LIDAR read to main view: {e}");
                    return
                }
            }
        })
    }
}

fn message<T: Send + std::fmt::Debug + 'static>(proxy: &MessageProxy<T>, msg: T, context: &str) {
    if let Err(e) = proxy.message(msg) {
        log::warn!("Failed to send message to main view ({context}): {e}");
    }
}

fn main() -> Result<()> {
    env_logger::init();

    let data = AppData::new()?;
    let app = Xilem::new_simple(data, AppData::display, WindowOptions::new("Operator Interface")
        .with_initial_inner_size(LogicalSize::new(1280, 800)));
    app.run_in(EventLoop::with_user_event())?;
    Ok(())
}
