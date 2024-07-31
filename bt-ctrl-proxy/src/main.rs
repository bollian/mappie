use bluer::rfcomm;
use clap::Parser;
use eyre::{Result, WrapErr};
use std::io::ErrorKind;
use std::time::Duration;

const TCP_RETRY_DURATION: Duration = Duration::from_secs(5);

#[derive(Parser, Clone, Debug)]
#[command(author, version, about)]
/// A proxy that first connects to the robot control software, then exposes that service over a
/// Bluetooth RFCOMM socket.
struct Args {
    /// Controllable robot service to connect to
    #[arg(short, long, default_value = "127.0.0.1:9090")]
    tcp: String,

    /// Bluetooth channel to listen on
    #[arg(short, long, default_value_t = 50)]
    channel: u8,

    #[arg(short, long, default_value = "Mappie Robot")]
    advertisement: String,
}

fn main() -> Result<()> {
    env_logger::init();
    let args = Args::parse();

    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let mut controllable = loop {
                match tokio::net::TcpStream::connect(args.tcp.as_str()).await {
                    Ok(c) => break c,
                    Err(e) => match e.kind() {
                        ErrorKind::Interrupted => continue, // immediately retry
                        ErrorKind::ConnectionReset | ErrorKind::ConnectionRefused | ErrorKind::ConnectionAborted => {
                            log::warn!("Controllable robot refused connection, retrying: {}", e);
                            continue
                        }
                        ErrorKind::NotFound | ErrorKind::TimedOut => {
                            log::info!("Connection timed out, retrying in {:?}", TCP_RETRY_DURATION);
                            tokio::time::sleep(TCP_RETRY_DURATION).await;
                        }
                        _ => {
                            return Err(eyre::Report::new(e).wrap_err("TCP connection to controllable service failed"))
                        }
                    }
                }
            };

            let bt_socket_addr = rfcomm::SocketAddr {
                addr: bluer::Address::any(),
                channel: args.channel,
            };

            let bt_socket = rfcomm::Listener::bind(bt_socket_addr).await
                .wrap_err("Failed to create bluetooth socket")?;
            log::trace!("Created bluetooth socket");

            let bluetooth = bluer::Session::new().await?;
            let adapter = bluetooth.default_adapter().await?;
            log::trace!("Got bluetooth adapter");
            let _advertisement = adapter.advertise(bluer::adv::Advertisement {
                advertisement_type: bluer::adv::Type::Peripheral,
                appearance: Some(0x0740),
                discoverable: Some(true),
                local_name: Some(args.advertisement.clone()),
                ..Default::default()
            }).await?;
            log::trace!("Advertising bluetooth service");

            let (mut controller, _addr) = bt_socket.accept().await
                .wrap_err("Failed to accept controller connection")?;
            log::trace!("Accepting controller connections");

            tokio::io::copy_bidirectional(&mut controllable, &mut controller).await
                .wrap_err("Stream ended unexpectedly")?;

            return Ok(());
        })
}
