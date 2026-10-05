//! The `mqtt-broker` CLI binary: it starts, serves real clients, honours its
//! limit flags, and shuts down cleanly on SIGTERM (what Docker, Kubernetes and
//! systemd send to stop a service).

#![cfg(unix)]

use std::net::TcpListener;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use mqtt_client::{ConnectOptions, MqttClient, MqttVersion, QoS};

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn spawn_broker(extra: &[&str]) -> (Child, u16) {
    let port = free_port();
    let child = Command::new(env!("CARGO_BIN_EXE_mqtt-broker"))
        .args([
            "--bind",
            "127.0.0.1",
            "--port",
            &port.to_string(),
            "--log-level",
            "warn",
        ])
        .args(extra)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("the broker binary must start");
    (child, port)
}

async fn wait_until_listening(port: u16) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .is_err()
    {
        assert!(Instant::now() < deadline, "broker never started listening");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn client(port: u16, id: &str) -> MqttClient {
    MqttClient::new(ConnectOptions::new("127.0.0.1", port, id, MqttVersion::V5))
}

#[tokio::test]
async fn serves_clients_and_exits_cleanly_on_sigterm() {
    let (mut child, port) = spawn_broker(&[]);
    wait_until_listening(port).await;

    let c = client(port, "cli-client");
    c.connect()
        .await
        .expect("a client can connect to the CLI broker");
    c.publish("cli/t".into(), b"x".to_vec(), QoS::AtLeastOnce, false)
        .await
        .expect("and publish at QoS 1");

    let status = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(status.success());

    let deadline = Instant::now() + Duration::from_secs(10);
    let exit = loop {
        if let Some(exit) = child.try_wait().unwrap() {
            break exit;
        }
        assert!(Instant::now() < deadline, "broker ignored SIGTERM");
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert!(
        exit.success(),
        "graceful shutdown must exit 0, got {exit:?}"
    );
}

#[tokio::test]
async fn limit_flags_are_applied() {
    let (mut child, port) = spawn_broker(&["--max-packet-size", "1024", "--max-clients", "1"]);
    wait_until_listening(port).await;

    let first = client(port, "first");
    first.connect().await.unwrap();
    // --max-clients 1: a second distinct client is refused.
    let err = client(port, "second").connect().await.unwrap_err();
    assert!(
        matches!(err, mqtt_client::MqttError::ConnectionRefused(_)),
        "{err:?}"
    );
    // --max-packet-size 1024: an oversized publish drops the connection.
    let _ = first
        .publish("big".into(), vec![0u8; 8192], QoS::AtMostOnce, false)
        .await;
    let deadline = Instant::now() + Duration::from_secs(5);
    while first.is_connected() {
        assert!(
            Instant::now() < deadline,
            "oversized packet must disconnect the sender"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    child.kill().unwrap();
    let _ = child.wait();
}

#[test]
fn help_and_version_work() {
    for flag in ["--help", "--version"] {
        let out = Command::new(env!("CARGO_BIN_EXE_mqtt-broker"))
            .arg(flag)
            .output()
            .unwrap();
        assert!(out.status.success(), "{flag}");
        assert!(!out.stdout.is_empty(), "{flag} prints something");
    }
}
