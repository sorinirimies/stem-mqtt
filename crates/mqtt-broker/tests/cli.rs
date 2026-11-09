//! The `mqtt-broker` CLI binary: it starts, serves real clients, honours its
//! limit flags, and shuts down cleanly on SIGTERM (what Docker, Kubernetes and
//! systemd send to stop a service).

#![cfg(unix)]
#![allow(clippy::unwrap_used)] // Test harness: a failed setup step should panic.
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use mqtt_client::{ConnectOptions, MqttClient, MqttVersion, QoS};

/// The port in the broker's "mqtt-broker listening" log line (not the WebSocket one).
fn listening_port(line: &str) -> Option<u16> {
    if !line.contains("mqtt-broker listening") || line.contains("WebSocket") {
        return None;
    }
    line.split_whitespace()
        .find_map(|word| word.strip_prefix("port="))?
        .parse()
        .ok()
}

/// Starts the broker on `--port 0` and returns once it listens, with the port the OS
/// picked, read from its log. (Picking a free port first and passing it raced: the
/// port was released before the broker bound it, and two tests' brokers could meet
/// on one port; the SIGTERM test then reached the `--max-clients 1` broker and was
/// reset.)
fn spawn_broker(extra: &[&str]) -> (Child, u16) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_mqtt-broker"))
        .args(["--bind", "127.0.0.1", "--port", "0", "--log-level", "info"])
        .args(extra)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("the broker binary must start");
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    // Keeps draining the log so the broker never blocks on a full pipe.
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Some(port) = listening_port(&line) {
                let _ = tx.send(port);
            }
        }
    });
    let port = rx
        .recv_timeout(Duration::from_secs(10))
        .expect("the broker logs the port it listens on");
    (child, port)
}

fn client(port: u16, id: &str) -> MqttClient {
    MqttClient::new(ConnectOptions::new("127.0.0.1", port, id, MqttVersion::V5))
}

#[tokio::test]
async fn serves_clients_and_exits_cleanly_on_sigterm() {
    let (mut child, port) = spawn_broker(&[]);

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

// Regression: the CLI logged the requested port, so `--port 0` logged `port=0`.
#[test]
fn port_zero_logs_the_port_actually_bound() {
    let (mut child, port) = spawn_broker(&[]);
    assert_ne!(port, 0);
    assert!(std::net::TcpStream::connect(("127.0.0.1", port)).is_ok());
    child.kill().unwrap();
    let _ = child.wait();
}

#[test]
fn listening_port_reads_the_mqtt_line_only() {
    let line =
        "2026-11-09T06:00:00Z  INFO mqtt_broker: mqtt-broker listening bind=127.0.0.1 port=51234";
    assert_eq!(listening_port(line), Some(51234));
    assert_eq!(
        listening_port(&line.replace("listening", "listening (WebSocket)")),
        None
    );
    assert_eq!(listening_port("INFO shutting down"), None);
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
