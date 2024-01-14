mod gui_framework;

use std::collections::HashMap;
use std::sync::mpsc;

use egui_winit::egui;
use eyre::Result;
use futures::{Stream, StreamExt};
use gui_framework::{App, GamepadInput};
use thiserror::Error;
use tokio::time::timeout;

#[derive(Copy, Clone, Debug)]
#[repr(u8)]
enum ExitCode {
    GraphicsInitFailed,
}

fn main() -> Result<()> {
    env_logger::init();
    gui_framework::run(OperatorInterfaceApp::new())
}

#[derive(Error, Debug)]
enum AppError {
    #[error("bluetooth setup failed")]
    BluetoothSetup(#[from] bluer::Error),
}

struct OperatorInterfaceApp {
    frame_count: usize,
    debug_layout: bool,
    bt_adapter: bluer::Adapter,
    state: AppState,
}

struct DeviceProps {
    name: String,
}

enum AppState {
    StartScreen {
        connection_options: HashMap<bluer::Address, Option<DeviceProps>>,
        selected_option: Option<bluer::Address>,
        adapter_rx: mpsc::Receiver<bluer::AdapterEvent>,
        device_rx: mpsc::Receiver<(bluer::Address, DeviceProps)>,
        device_tx: mpsc::Sender<(bluer::Address, DeviceProps)>,
    },
    Connected {
        conn: bluer::rfcomm::Stream,
    },
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
            debug_layout: false,
            frame_count: 0,
            bt_adapter,
            state: AppState::StartScreen {
                connection_options: HashMap::new(),
                selected_option: None,
                adapter_rx,
                device_rx,
                device_tx,
            },
        })
    }
}

impl App for OperatorInterfaceApp {
    type Err = AppError;

    fn update(&mut self, ctx: &egui::Context, gamepad: GamepadInput) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.checkbox(&mut self.debug_layout, "Debug on hover");
            ctx.set_debug_on_hover(self.debug_layout);

            match &mut self.state {
                AppState::StartScreen { connection_options, selected_option, adapter_rx, device_rx, device_tx } => {
                    for event in adapter_rx.try_iter() {
                        match event {
                            bluer::AdapterEvent::DeviceAdded(address) => {
                                if let Ok(device) = self.bt_adapter.device(address) {
                                    let tx = device_tx.clone();
                                    gui_framework::spawn(
                                        read_device_props(device, tx)
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

                    if let Some((&default_selection, _)) = connection_options.iter().next() {
                        let selected_option = selected_option.get_or_insert(default_selection);
                        let selected_text = connection_options[selected_option]
                            .as_ref()
                            .map(|p| p.name.clone())
                            .unwrap_or_else(|| selected_option.to_string());

                        egui::ComboBox::from_label("Select a device")
                            .selected_text(selected_text)
                            .show_ui(ui, |ui| {
                                for (&address, props) in connection_options.iter() {
                                    let name = props.as_ref().map(|p| p.name.clone())
                                        .unwrap_or_else(|| address.to_string());
                                    ui.selectable_value(selected_option, address, name);
                                }
                            });

                        if ui.button("Connect").clicked() {
                            ui.label("Connecting!");
                        }
                    } else {
                        ui.label("No devices available for connection");
                    }
                }
                AppState::Connected { conn } => {

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
        });
        self.frame_count += 1;
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
