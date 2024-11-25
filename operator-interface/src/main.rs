mod gui_framework;
mod oi;

use std::net::{IpAddr, SocketAddr};

use egui_winit::egui;
use eyre::Result;
use gui_framework::{App, GamepadInput};
use thiserror::Error;
use tokio::net::UdpSocket;
// use tokio::net::TcpStream;
use tokio::task::JoinHandle;
use tokio::time::timeout;

fn main() -> Result<()> {
    env_logger::init();
    gui_framework::run(OperatorInterfaceApp::new())
}

#[derive(Error, Debug)]
enum AppError {
}

struct OperatorInterfaceApp {
    frame_count: usize,
    debug_layout: bool,
    current_error: Option<Banner>,
    state: AppState,
}

impl OperatorInterfaceApp {
    async fn new() -> Result<Self, AppError> {
        Ok(Self {
            debug_layout: false,
            frame_count: 0,
            current_error: None,
            state: AppState::Connecting {
                error: None,
                address: "rpi:9090".to_string(),
                port: 9090,
                connecting_task: None,
            },
        })
    }
}

enum AppState {
    Connecting {
        error: Option<Banner>,
        address: String,
        port: u16,
        connecting_task: Option<JoinHandle<Result<UdpSocket>>>,
    },
    OperatorInterface(oi::OperatorInterface),
    FallbackError(eyre::Report),
}

impl App for OperatorInterfaceApp {
    type Err = AppError;

    fn update(&mut self, ctx: &egui::Context, gamepad: GamepadInput) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.checkbox(&mut self.debug_layout, "Debug on hover");
            // ctx.set_debug_on_hover(self.debug_layout);

            if let Some(e) = &self.current_error {
                e.draw(ui);
            }

            let mut next_state = None;

            'appstate: {
                match &mut self.state {
                    AppState::Connecting {
                        error, address, port, connecting_task
                    } => {
                        if let Some(banner) = error {
                            banner.draw(ui);
                        }

                        ui.add_enabled_ui(connecting_task.is_none(), |ui| {
                            ui.with_layout(egui::Layout::left_to_right(egui::Align::TOP), |ui| {
                                ui.label("Address: ");
                                ui.text_edit_singleline(address);
                                ui.label("Port: ");
                                ui.add(egui::widgets::DragValue::new(port).speed(1));
                            });

                            if connecting_task.is_some() {
                                ui.label("Attempting connection...");
                            }

                            if ui.add(egui::Button::new("Connect")).clicked() {
                                *connecting_task = Some(gui_framework::spawn(
                                        connect_to_robot(address.clone(), *port)));
                            }
                        });

                        if connecting_task.is_some() {
                            ui.with_layout(egui::Layout::left_to_right(egui::Align::TOP), |ui| {
                                ui.label("Attempting connection...");
                                ui.spinner();
                            });
                        }

                        ui.label(format!("Left stick: {:?}", gamepad.left_stick));
                        ui.label(format!("Right stick: {:?}", gamepad.right_stick));

                        ui.label(format!("DPad Down: {}", gamepad.dpad_down));
                        ui.label(format!("DPad Right: {}", gamepad.dpad_right));
                        ui.label(format!("DPad Up: {}", gamepad.dpad_up));
                        ui.label(format!("DPad Left: {}", gamepad.dpad_left));

                        let stream = if let Some(task) = connecting_task.take() {
                            if task.is_finished() {
                                gui_framework::block_on(task)
                            } else {
                                *connecting_task = Some(task);
                                break 'appstate
                            }
                        } else { break 'appstate };

                        match stream {
                            Ok(Ok(s)) => {
                                next_state = Some(AppState::OperatorInterface(oi::OperatorInterface::new(s)));
                            },
                            Err(e) => {
                                *error = Some(Banner::Error(eyre::Report::new(e)
                                    .wrap_err("Connection task got cancelled")));
                            }
                            Ok(Err(e)) => {
                                *error = Some(Banner::Error(eyre::eyre!("Network connection failed: {}", e)));
                            }
                        }
                    }
                    AppState::OperatorInterface(oi) => {
                        oi.draw(ui, gamepad);
                    }
                    AppState::FallbackError(e) => {
                        ui.label(format!("Uh oh! Fallback error: {}", e));
                    }
                }
            }

            if let Some(next) = next_state {
                self.state = next;
            }

            ui.heading("Hello, Mappie 2!");
            ui.label(format!("Frame count: {}", self.frame_count));
        });

        self.frame_count += 1;
    }
}

async fn connect_to_robot(addr: String, port: u16) -> Result<UdpSocket> {
    let sockaddr = match addr.parse::<IpAddr>() {
        Ok(a) => SocketAddr::new(a, port),
        Err(_) => {
            match tokio::net::lookup_host(addr.as_str()).await {
                Ok(mut iter) => match iter.next() {
                    Some(a) => a,
                    None => eyre::bail!("Failed to find host '{}'", addr.as_str()),
                }
                Err(e) => {
                    return Err(eyre::Report::new(e)
                        .wrap_err("Host lookup failed"))
                }
            }
        }
    };

    let conn: Result<UdpSocket> = timeout(std::time::Duration::from_secs(30),
        async {
            let sock = UdpSocket::bind("0.0.0.0:9090").await?;
            sock.connect(sockaddr).await?;
            Ok(sock)
        }).await?;
    Ok(conn?)
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
