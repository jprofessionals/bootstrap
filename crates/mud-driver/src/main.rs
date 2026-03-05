use std::path::Path;

use anyhow::Result;
use clap::Parser;
use tracing_subscriber::EnvFilter;

use mud_driver::config::Config;
use mud_driver::server::Server;

/// MUD driver — the core game engine.
#[derive(Parser, Debug)]
#[command(name = "mud-driver", version, about)]
struct Args {
    /// Path to the configuration file.
    #[arg(short, long, default_value = "config.yml")]
    config: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();
    let config = Config::load(Path::new(&args.config))?;

    let mut server = Server::new(config);
    server.boot().await
}
