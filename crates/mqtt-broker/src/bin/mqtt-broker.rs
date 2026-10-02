//! Standalone MQTT broker server binary.
//!
//! ```sh
//! mqtt-broker --bind 0.0.0.0 --port 1883
//! ```

use clap::Parser;
use mqtt_broker::{MqttBroker, MqttBrokerConfig};

/// An MQTT 3.1.1 / MQTT 5.0 broker.
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

    /// Largest packet accepted from a client, in bytes (0 = built-in
    /// default, 1 MiB). Bigger packets disconnect the sender.
    #[arg(long, default_value_t = 0)]
    max_packet_size: u32,

    /// Packets queued per client socket before further deliveries to that
    /// client are dropped (0 = built-in default, 4096).
    #[arg(long, default_value_t = 0)]
    max_outbound_queue: u32,

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
    config.max_packet_size = args.max_packet_size;
    config.max_outbound_queue = args.max_outbound_queue;

    let broker = MqttBroker::new(config);
    broker
        .start()
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;

    tracing::info!(bind = %args.bind, port = args.port, "mqtt-broker listening");
    if let Some(ws_port) = args.ws_port {
        tracing::info!(bind = %args.bind, port = ws_port, "mqtt-broker listening (WebSocket)");
    }

    // Run until interrupted (Ctrl-C) or terminated (SIGTERM — what Docker,
    // Kubernetes and systemd send to stop a service).
    shutdown_signal().await?;
    tracing::info!("shutting down");
    broker
        .stop()
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    Ok(())
}

/// Resolves on Ctrl-C, or — on Unix — SIGTERM.
async fn shutdown_signal() -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut terminate = signal(SignalKind::terminate())?;
        tokio::select! {
            result = tokio::signal::ctrl_c() => result,
            _ = terminate.recv() => Ok(()),
        }
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await
    }
}
