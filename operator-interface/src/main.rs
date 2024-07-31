mod gui_framework;

use std::collections::HashMap;
use std::sync::mpsc;

use bluer::rfcomm;
use egui_winit::egui;
use eyre::{Context, Result};
use futures::{Stream, StreamExt};
use gui_framework::{App, GamepadInput};
use thiserror::Error;
use tokio::task::JoinHandle;
use tokio::time::timeout;

fn main() -> Result<()> {
    env_logger::init();
    gui_framework::run(OperatorInterfaceApp::new())
}

#[derive(Error, Debug)]
enum AppError {
    #[error("Bluetooth setup failed")]
    BluetoothSetup(#[from] bluer::Error),
    #[error("Bluetooth I/O: {0:?}")]
    BluetoothIo(std::io::Error),
}

struct OperatorInterfaceApp {
    frame_count: usize,
    debug_layout: bool,
    bt_adapter: bluer::Adapter,
    current_error: Option<Banner>,
    state: AppState,
}

impl OperatorInterfaceApp {
    async fn new() -> Result<Self, AppError> {
        let bluetooth = bluer::Session::new().await?;
        let bt_adapter = bluetooth.default_adapter().await?;

        let devices = bt_adapter.discover_devices().await?;
        let (adapter_tx, adapter_rx) = mpsc::channel();
        gui_framework::spawn(load_devices(devices, adapter_tx));

        let (device_tx, device_rx) = mpsc::channel();

        Ok(Self {
            bt_adapter,
            debug_layout: false,
            frame_count: 0,
            current_error: None,
            state: AppState::Connecting {
                connection_options: HashMap::new(),
                channel: 1,
                adapter_rx,
                device_tx,
                device_rx,
                state: ConnectingState::SelectionScreen {
                    selected_option: None,
                }
            },
        })
    }
}

struct DeviceProps {
    name: String,
}

enum AppState {
    Connecting {
        connection_options: HashMap<bluer::Address, Option<DeviceProps>>,
        channel: u8,
        adapter_rx: mpsc::Receiver<bluer::AdapterEvent>,
        device_tx: mpsc::Sender<(bluer::Address, DeviceProps)>,
        device_rx: mpsc::Receiver<(bluer::Address, DeviceProps)>,
        state: ConnectingState,
    },
    OperatorInterface {
        robot_addr: bluer::Address,
        robot_props: Option<DeviceProps>,
        connection: bluer::rfcomm::Stream,
    },
    FallbackError(eyre::Report),
}

enum ConnectingState {
    SelectionScreen {
        selected_option: Option<bluer::Address>,
    },
    Connecting {
        connecting_task: Option<JoinHandle<Result<rfcomm::Stream>>>,
        robot_addr: bluer::Address,
        robot_props: Option<DeviceProps>,
    }
}

impl App for OperatorInterfaceApp {
    type Err = AppError;

    fn update(&mut self, ctx: &egui::Context, gamepad: GamepadInput) {
        let mut next_state = None;

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.checkbox(&mut self.debug_layout, "Debug on hover");
            ctx.set_debug_on_hover(self.debug_layout);

            if let Some(e) = &self.current_error {
                e.draw(ui);
            }

            match &mut self.state {
                AppState::Connecting {
                    connection_options,
                    channel,
                    adapter_rx,
                    device_tx,
                    device_rx,
                    state: connecting_state
                } => {
                    let mut next_connecting_state = None;

                    match connecting_state {
                        ConnectingState::SelectionScreen { selected_option } => {
                            for event in adapter_rx.try_iter() {
                                match event {
                                    bluer::AdapterEvent::DeviceAdded(address) => {
                                        if let Ok(device) = self.bt_adapter.device(address) {
                                            gui_framework::spawn(
                                                read_device_props(device, device_tx.clone())
                                            );
                                            connection_options.insert(address, None);
                                        }
                                    }
                                    bluer::AdapterEvent::DeviceRemoved(address) => {
                                        connection_options.remove(&address);
                                    }
                                    bluer::AdapterEvent::PropertyChanged(..) => {}
                                }
                            }

                            for (address, props) in device_rx.try_iter() {
                                connection_options.insert(address, Some(props));
                            }

                            if let Some((default_selection, _)) = connection_options.iter().next() {
                                let selected_addr = if let Some(selection) = selected_option {
                                    selection
                                } else {
                                    selected_option.insert(*default_selection)
                                };
                                let (selected_addr, selected_props) = if let Some(props) = connection_options.get(selected_addr) {
                                    (selected_addr, props)
                                } else {
                                    // since items might be removed from the list, we need to reset
                                    // the selected_option back to default if we don't find it in the map
                                    (selected_option.insert(*default_selection), &connection_options[default_selection])
                                };
                                let button_text = if let Some(props) = selected_props {
                                    props.name.clone()
                                } else {
                                    selected_addr.to_string()
                                };

                                egui::ComboBox::from_label("Select a device")
                                    .selected_text(button_text)
                                    .show_ui(ui, |ui| {
                                        for (&address, props) in connection_options.iter() {
                                            let name = props.as_ref().map(|p| p.name.clone())
                                                .unwrap_or_else(|| address.to_string());
                                            ui.selectable_value(selected_addr, address, name);
                                        }
                                    });
                                ui.add(egui::DragValue::new(channel));

                                if ui.button("Connect").clicked() {
                                    let connecting_task = gui_framework::spawn(
                                        connect_to_robot(*selected_addr, *channel));

                                    let (_, selected_props) = connection_options.remove_entry(selected_addr)
                                        .expect("Missing known good connection option");
                                    next_connecting_state = Some(ConnectingState::Connecting {
                                        robot_addr: *selected_addr,
                                        robot_props: selected_props,
                                        connecting_task: Some(connecting_task),
                                    });
                                }
                            } else {
                                ui.label("No devices available for connection");
                            }
                        }
                        ConnectingState::Connecting { connecting_task, robot_addr, robot_props } => {
                            ui.label(format!("Connecting to {}...", robot_addr));

                            if let Some(connecting_task) = connecting_task {
                                if connecting_task.is_finished() {
                                    let robot_dev = gui_framework::block_on(connecting_task);
                                    let robot_dev = match robot_dev {
                                        Ok(conn) => conn.context("Error during connection"),
                                        Err(e) => Err(eyre::Report::new(e).wrap_err("Connection task died")),
                                    };
                                    match robot_dev {
                                        Ok(conn) => {
                                            next_state = Some(AppState::OperatorInterface {
                                                connection: conn,
                                                robot_addr: *robot_addr,
                                                robot_props: robot_props.take(),
                                            });
                                        },
                                        Err(e) => {
                                            self.current_error = Some(Banner::Error(e));
                                            next_connecting_state = Some(ConnectingState::SelectionScreen {
                                                selected_option: None,
                                            });
                                        },
                                    };
                                }
                            } else { panic!("Connecting task is missing") }
                        }
                    }

                    if let Some(next) = next_connecting_state {
                        *connecting_state = next;
                    }
                }
                AppState::OperatorInterface { robot_addr, robot_props, connection: _ } => {
                    let conn_desc = if let Some(props) = robot_props {
                        format!("{} ({})", props.name, robot_addr)
                    } else { robot_addr.to_string() };
                    ui.label(format!("Connected to {}", conn_desc));
                }
                AppState::FallbackError(e) => {
                    ui.label(format!("Uh oh! Fallback error: {}", e));
                }
            }

            ui.heading("Hello, Mappie 2!");
            ui.label(format!("Frame count: {}", self.frame_count));
            ui.label(format!("Left stick: {:?}", gamepad.left_stick));
            ui.label(format!("Right stick: {:?}", gamepad.right_stick));

            ui.label(format!("DPad Down: {}", gamepad.dpad_down));
            ui.label(format!("DPad Right: {}", gamepad.dpad_right));
            ui.label(format!("DPad Up: {}", gamepad.dpad_up));
            ui.label(format!("DPad Left: {}", gamepad.dpad_left));

            if let Some(next_state) = next_state {
                self.state = next_state;
            }
        });

        self.frame_count += 1;
    }
}

#[derive(Debug)]
enum Banner {
    Warning(eyre::Report),
    Error(eyre::Report),
}

impl Banner {
    fn draw(&self, ui: &mut egui::Ui) {
        match self {
            Banner::Warning(e) => {
                ui.label(egui::RichText::new(format!("Warning: {e:?}"))
                    .color(egui::Color32::from_rgb(255, 100, 0)));
            },
            Banner::Error(e) => {
                ui.label(egui::RichText::new(format!("Critical Error: {e:?}"))
                    .color(egui::Color32::RED));
            },
        }
    }
}

async fn load_devices<S>(mut devices: S, tx: mpsc::Sender<bluer::AdapterEvent>)
where
    S: Stream<Item = bluer::AdapterEvent> + Unpin,
{
    log::trace!("Entering load_devices task");

    loop {
        let event = timeout(std::time::Duration::from_secs(1), devices.next());
        let event = match event.await {
            Ok(Some(event)) => event,
            Err(_) => continue, // timeout, retry
            Ok(None) => break, // no more events
        };

        if let Err(mpsc::SendError(_event)) = tx.send(event) {
            log::debug!(
                "Channel to GUI thread broken. Assuming connection made and ending enumeration task"
            );
            return
        }

        gui_framework::request_redraw();
    }

    log::debug!("Exiting load_devices task");
}

async fn read_device_props(device: bluer::Device, tx: mpsc::Sender<(bluer::Address, DeviceProps)>) {
    log::trace!("Reading properties from BT device {}", device.address());

    if let Ok(props) = device.all_properties().await {
        let mut name = None;
        for p in props {
            match p {
                bluer::DeviceProperty::Alias(alias) => name = Some(alias),
                _ => {}
            }
        }

        let props = match name {
            Some(name) => DeviceProps { name },
            None => return,
        };

        if let Err(_) = tx.send((device.address(), props)) {
            log::debug!(
                "Channel to GUI thread broken. Assuming connection made and ending enumeration task"
            );
            return
        }

        gui_framework::request_redraw();
    }
}

async fn connect_to_robot(addr: bluer::Address, channel: u8) -> Result<rfcomm::Stream> {
    let sock_addr = rfcomm::SocketAddr { addr, channel };
    let sock = rfcomm::Socket::new()
        .map_err(AppError::BluetoothIo)
        .wrap_err("Failed to create socket")?;
    sock.connect(sock_addr).await
        .map_err(AppError::BluetoothIo)
        .wrap_err_with(|| format!("Failed to connect BT socket to address {}", sock_addr))
}
