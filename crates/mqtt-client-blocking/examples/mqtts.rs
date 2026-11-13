//! The `mqtt` example over TLS (needs `--features rustls`): watch or drive devices on a TLS port.
//!
//! ```text
//! cargo run -p stem-mqtt-client-blocking --features rustls --example mqtts -- sub broker.local 8883 'home/#' [ca.pem]
//! cargo run -p stem-mqtt-client-blocking --features rustls --example mqtts -- pub broker.local 8883 home/kitchen/light on [ca.pem]
//! ```
//!
//! Without `ca.pem` the broker's certificate must chain to a public CA; with it, to that CA (or be
//! that self-signed certificate).

use std::time::Duration;

use mqtt_client_blocking::{Client, Options, QoS, tls};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let usage = "usage: mqtts sub <host> <port> <filter> [ca.pem]  |  mqtts pub <host> <port> <topic> <payload> [ca.pem]";
    let arg = |i: usize| args.get(i).ok_or(usage);
    let (mode, host, port) = (arg(0)?.as_str(), arg(1)?, arg(2)?.parse::<u16>()?);
    let ca_index = if mode == "pub" { 5 } else { 4 };
    let ca = args.get(ca_index).map(std::fs::read).transpose()?;

    let config = tls::client_config(ca.as_deref(), None)?;
    let stream = tls::connect(host, port, config)?;
    let mut client = Client::connect(
        stream,
        Options::new(format!("mqtts-example-{}", std::process::id())),
    )?;
    match mode {
        "sub" => {
            client.subscribe(arg(3)?, QoS::AtLeastOnce)?;
            eprintln!("subscribed over TLS; waiting for messages (Ctrl+C to stop)");
            loop {
                if let Some(m) = client.poll(Duration::from_millis(500))? {
                    println!("{} {}", m.topic, String::from_utf8_lossy(&m.payload));
                }
            }
        }
        "pub" => {
            client.publish(arg(3)?, arg(4)?.as_bytes(), QoS::AtLeastOnce, false)?;
            client.disconnect()?;
            Ok(())
        }
        _ => Err(usage.into()),
    }
}
