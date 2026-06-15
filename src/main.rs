mod audio;
mod client;
mod protocol;
mod server;
mod utils;

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "audio-relay")]
#[command(about = "A Rust clone of AudioRelay - stream audio between PC and phone")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Server {
        #[arg(short, long, default_value = "0.0.0.0")]
        host: String,
        #[arg(short, long, default_value = "8080")]
        port: u16,
        #[arg(long, default_value = "8081")]
        web_port: u16,
    },
    Client {
        #[arg(short, long)]
        server: String,
        #[arg(short, long, default_value = "8080")]
        port: u16,
        #[arg(short, long, value_enum, default_value = "speaker")]
        mode: ClientMode,
    },
}

#[derive(clap::ValueEnum, Clone, PartialEq)]
enum ClientMode {
    Speaker,
    Mic,
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init_from_env(env_logger::Env::default().default_filter_or("info"));

    let cli = Cli::parse();

    match cli.command {
        Commands::Server { host, port, web_port } => {
            log::info!("Starting AudioRelay Server on {}:{}", host, port);
            log::info!("Management UI at http://{}:{}", host, web_port);
            server::run_server(&host, port, web_port).await?;
        }
        Commands::Client {
            server,
            port,
            mode,
        } => {
            let mode_str = match mode {
                ClientMode::Speaker => "speaker",
                ClientMode::Mic => "mic",
            };
            log::info!(
                "Connecting to server {}:{} in {} mode",
                server,
                port,
                mode_str
            );
            client::run_client(&server, port, mode == ClientMode::Mic).await?;
        }
    }

    Ok(())
}
