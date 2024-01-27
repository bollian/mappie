use clap::Parser;
use eyre::Result;

#[derive(Parser, Clone, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Controllable robot service to connect to
    #[arg(short, long, default_value = "0.0.0.0:9090")]
    ctrl_addr: String,
}

fn main() -> Result<()> {
    env_logger::init();

    smol::block_on(async {
        let control_socket = smol::net::TcpListener::bind("0.0.0.0:9090").await?;

        loop {
            let (controller, _addr) = control_socket.accept().await?
        }

        return Ok(())
    })
}
