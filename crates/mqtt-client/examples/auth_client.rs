//! Authentication from the client side: one login is rejected, another accepted.
//!
//! Start the authenticating broker first:
//! `cargo run -p stem-mqtt-broker --example auth_broker`
//!
//! Then: `cargo run -p stem-mqtt-client --example auth_client`
//!
//! `auth_broker` only accepts `demo` / `demo`; every attempt (and every connect, disconnect and
//! publish) is logged on the broker's side.

use mqtt_client::{ConnectOptions, MqttClient, MqttVersion, QoS};

fn with_login(client_id: &str, user: &str, password: &str) -> ConnectOptions {
    let mut options = ConnectOptions::new("127.0.0.1", 1883, client_id, MqttVersion::V5);
    options.username = Some(user.into());
    options.password = Some(password.as_bytes().to_vec());
    options
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("trying intruder / guess ...");
    let intruder = MqttClient::new(with_login("intruder", "intruder", "guess"));
    match intruder.connect().await {
        Ok(_) => println!("  unexpectedly accepted"),
        Err(e) => println!("  rejected: {e}"),
    }

    println!("trying demo / demo ...");
    let client = MqttClient::new(with_login("alice", "demo", "demo"));
    client.connect().await?;
    println!("  accepted");
    client
        .publish(
            "stem-mqtt/examples/auth".into(),
            b"hello from alice".to_vec(),
            QoS::AtLeastOnce,
            false,
        )
        .await?;
    println!("  published a message");
    client.disconnect().await?;
    Ok(())
}
