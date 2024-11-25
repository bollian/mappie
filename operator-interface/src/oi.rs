use crate::gui_framework::GamepadInput;
use messages::Move;
use std::io::ErrorKind;
// use tokio::net::TcpStream;
use tokio::net::UdpSocket;

pub struct OperatorInterface {
    pub connection: UdpSocket,
    pub buffer: [u8; messages::Move::POSTCARD_COBS_BUFFER_MAX_SIZE],
    pub skipped_msgs_count: u32,
}

impl OperatorInterface {
    pub fn new(connection: UdpSocket) -> Self {
        Self {
            connection,
            buffer: [0; messages::Move::POSTCARD_COBS_BUFFER_MAX_SIZE],
            skipped_msgs_count: 0,
        }
    }

    pub fn draw(&mut self, ui: &mut egui::Ui, gamepad: GamepadInput) {
        let msg = Move {
            translate: [gamepad.left_stick.0, gamepad.left_stick.1].into(),
            rotate: gamepad.right_stick.0,
        };
        let msg_bytes = match postcard::to_slice_cobs(&msg, &mut self.buffer[..]) {
            Ok(slice) => slice,
            Err(e) => {
                panic!("Unexpected serialization error ({}) on message: {:?}", e, msg)
            }
        };

        match self.connection.try_send(msg_bytes) {
            Ok(_) => {}
            Err(e) => match e.kind() {
                ErrorKind::WouldBlock => {
                    self.skipped_msgs_count += 1;
                }
                ErrorKind::BrokenPipe | ErrorKind::ConnectionReset => {
                    log::error!("Broken pipe! Fix me!");
                }
                _ => {}
            }
        }

        ui.label(format!("Skipped messages: {}", self.skipped_msgs_count));
        ui.label(format!("Left stick: {:?}", gamepad.left_stick));
        ui.label(format!("Right stick: {:?}", gamepad.right_stick));

        ui.label(format!("DPad Down: {}", gamepad.dpad_down));
        ui.label(format!("DPad Right: {}", gamepad.dpad_right));
        ui.label(format!("DPad Up: {}", gamepad.dpad_up));
        ui.label(format!("DPad Left: {}", gamepad.dpad_left));
    }
}
