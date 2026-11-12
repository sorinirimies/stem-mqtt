//! A tiny command-line client, to watch and drive devices:
//!
//! ```text
//! cargo run -p stem-mqtt-client-blocking --example mqtt -- sub 192.168.1.23:1883 'weathertrack/#'
//! cargo run -p stem-mqtt-client-blocking --example mqtt -- pub 192.168.1.23:1883 weathertrack/c6-a1b2/command refresh
//! ```

use std::net::TcpStream;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use mqtt_client_blocking::{Client, Options, QoS};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let usage = "usage: mqtt sub <host:port> <filter>  |  mqtt pub <host:port> <topic> <payload> [--retain]";
    let (mode, addr) = (
        args.first().ok_or(usage)?.as_str(),
        args.get(1).ok_or(usage)?,
    );
    let id = format!(
        "mqtt-example-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .subsec_millis()
    );
    let mut client = Client::connect(TcpStream::connect(addr)?, Options::new(id))?;
    match mode {
        "sub" => {
            client.subscribe(args.get(2).ok_or(usage)?, QoS::AtLeastOnce)?;
            eprintln!("subscribed; waiting for messages (Ctrl+C to stop)");
            loop {
                if let Some(m) = client.poll(Duration::from_millis(500))? {
                    println!(
                        "{}{} {}",
                        if m.retain { "[retained] " } else { "" },
                        m.topic,
                        String::from_utf8_lossy(&m.payload)
                    );
                }
            }
        }
        "pub" => {
            let retain = args.iter().any(|a| a == "--retain");
            client.publish(
                args.get(2).ok_or(usage)?,
                args.get(3).ok_or(usage)?.as_bytes(),
                QoS::AtLeastOnce,
                retain,
            )?;
            client.disconnect()?;
            Ok(())
        }
        _ => Err(usage.into()),
    }
}
