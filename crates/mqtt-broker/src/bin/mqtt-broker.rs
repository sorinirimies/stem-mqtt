//! Standalone MQTT broker server binary.
//!
//! ```sh
//! mqtt-broker --bind 0.0.0.0 --port 1883
//! ```

use clap::Parser;
use mqtt_broker::{MqttBroker, MqttBrokerConfig};

/// A full MQTT 3.1.1 / MQTT 5.0 broker.
#[derive(Parser, Debug)]
#[command(name = "mqtt-broker", version, about)]
struct Args {
    /// Address to bind the listening socket to.
    #[arg(long, default_value = "0.0.0.0")]
    bind: String,

    /// TCP port to listen on.
    #[arg(long, default_value_t = 1883)]
    port: u16,

    /// Allow clients to connect without a username/password.
    #[arg(long, default_value_t = true)]
    allow_anonymous: bool,

    /// Maximum simultaneously connected clients (0 = unlimited).
    #[arg(long, default_value_t = 0)]
    max_clients: u32,

    /// Port to also accept MQTT-over-WebSocket connections on — the
    /// transport a browser-based client (e.g. the demo webpage under
    /// `demo/web/`) must use, since browsers can't open raw TCP sockets.
    /// Omit to disable the WebSocket listener (default).
    #[arg(long)]
    ws_port: Option<u16>,

    /// Log verbosity: error, warn, info, debug, trace.
    #[arg(long, default_value = "info")]
    log_level: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(&args.log_level))
        .init();

    let mut config = MqttBrokerConfig::new(args.bind.clone(), args.port);
    config.allow_anonymous = args.allow_anonymous;
    config.max_clients = args.max_clients;
    config.ws_port = args.ws_port;

    let broker = MqttBroker::new(config);
    broker
        .start()
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;

    tracing::info!(bind = %args.bind, port = args.port, "mqtt-broker listening");
    if let Some(ws_port) = args.ws_port {
        tracing::info!(bind = %args.bind, port = ws_port, "mqtt-broker listening (WebSocket)");
    }

    // Run until interrupted.
    tokio::signal::ctrl_c().await?;
    tracing::info!("shutting down");
    broker
        .stop()
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    Ok(())
}
