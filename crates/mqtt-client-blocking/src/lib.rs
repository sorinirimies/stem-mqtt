//! A small **blocking** MQTT client on top of [`stem-mqtt-proto`](https://docs.rs/stem-mqtt-proto).
//!
//! No async runtime, no TLS stack of its own: it drives the codec over any [`Transport`] (a
//! `std::net::TcpStream` on a PC or on an ESP32 with ESP-IDF `std`, a TLS stream, a test double).
//! The caller owns the loop: call [`Client::poll`] regularly, it receives messages and keeps the
//! connection alive (PINGREQ) by itself.
//!
//! ```no_run
//! use std::net::TcpStream;
//! use std::time::Duration;
//! use mqtt_client_blocking::{Client, Options, QoS};
//!
//! let stream = TcpStream::connect("192.168.1.10:1883")?;
//! let mut client = Client::connect(stream, Options::new("weather-c6"))?;
//! client.subscribe("home/+/command", QoS::AtLeastOnce)?;
//! client.publish("home/c6/status", b"online", QoS::AtLeastOnce, true)?;
//! loop {
//!     if let Some(msg) = client.poll(Duration::from_millis(500))? {
//!         println!("{}: {}", msg.topic, String::from_utf8_lossy(&msg.payload));
//!     }
//! }
//! # Ok::<(), mqtt_client_blocking::Error>(())
//! ```
//!
//! Scope: QoS 0 and 1 for publishing, QoS 0, 1 and 2 for receiving (a QoS 2 message is delivered
//! once, when the PUBLISH arrives, and completed with PUBREC/PUBCOMP). Publishing at QoS 2 is
//! refused with [`Error::Unsupported`]. Reconnecting is the caller's job: after an error, drop the
//! client and connect again.

#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;

use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use bytes::{Bytes, BytesMut};
use mqtt_proto::MqttError;
use mqtt_proto::ack::SimpleAck;
use mqtt_proto::connect::{ConnectPacket, Will};
use mqtt_proto::packet::{DisconnectPacket, Packet};
use mqtt_proto::properties::Properties;
use mqtt_proto::publish::PublishPacket;
use mqtt_proto::subscribe::{SubscribeFilter, SubscribePacket, UnsubscribePacket};
pub use mqtt_proto::{MqttVersion, QoS};

/// How long [`Client::connect`], [`Client::publish`] at QoS 1 and [`Client::subscribe`] wait for
/// the broker's answer.
pub const DEFAULT_ACK_TIMEOUT: Duration = Duration::from_secs(5);

/// A byte stream with a read timeout, so that [`Client::poll`] can wait a bounded time.
pub trait Transport: Read + Write {
    /// Make reads fail with `WouldBlock` / `TimedOut` after `timeout` (None: block).
    fn set_read_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()>;
}

impl Transport for TcpStream {
    fn set_read_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        TcpStream::set_read_timeout(self, timeout)
    }
}

/// A message the client sends if the connection is lost without a DISCONNECT.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WillMessage {
    pub topic: String,
    pub payload: Vec<u8>,
    pub qos: QoS,
    pub retain: bool,
}

/// Connection settings.
#[derive(Debug, Clone)]
pub struct Options {
    pub client_id: String,
    pub username: Option<String>,
    pub password: Option<Vec<u8>>,
    /// Protocol version (default 3.1.1).
    pub version: MqttVersion,
    /// Keep-alive announced to the broker, in whole seconds (0 turns it off). Default 30 s.
    pub keep_alive: Duration,
    /// Start with a clean session (default true).
    pub clean_session: bool,
    pub will: Option<WillMessage>,
    /// How often to send PINGREQ when idle (default: half the keep-alive).
    pub ping_interval: Option<Duration>,
    /// How long to wait for CONNACK, PUBACK and SUBACK (default [`DEFAULT_ACK_TIMEOUT`]).
    pub ack_timeout: Duration,
}

impl Options {
    pub fn new(client_id: impl Into<String>) -> Self {
        Self {
            client_id: client_id.into(),
            username: None,
            password: None,
            version: MqttVersion::V311,
            keep_alive: Duration::from_secs(30),
            clean_session: true,
            will: None,
            ping_interval: None,
            ack_timeout: DEFAULT_ACK_TIMEOUT,
        }
    }

    pub fn credentials(
        mut self,
        username: impl Into<String>,
        password: impl Into<Vec<u8>>,
    ) -> Self {
        self.username = Some(username.into());
        self.password = Some(password.into());
        self
    }

    pub fn will(mut self, will: WillMessage) -> Self {
        self.will = Some(will);
        self
    }
}

/// A message received from the broker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub topic: String,
    pub payload: Vec<u8>,
    pub qos: QoS,
    pub retain: bool,
}

/// What went wrong.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("i/o error: {0}")]
    Io(#[from] io::Error),
    #[error("protocol error: {0}")]
    Protocol(#[from] MqttError),
    /// The broker refused the connection (the CONNACK reason code).
    #[error("connection refused by the broker (code {0})")]
    Refused(u8),
    /// The connection was closed by the broker (or the DISCONNECT packet came).
    #[error("connection closed")]
    Closed,
    /// No answer in time (what was awaited).
    #[error("timed out waiting for {0}")]
    Timeout(&'static str),
    /// The broker answered with something that does not fit (what happened).
    #[error("unexpected answer: {0}")]
    Unexpected(&'static str),
    /// A feature this client does not have (publishing at QoS 2).
    #[error("not supported: {0}")]
    Unsupported(&'static str),
    /// The broker rejected a subscription.
    #[error("subscription refused (code {0})")]
    SubscribeRefused(u8),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Traffic counters (for a status screen).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Stats {
    pub packets_tx: u64,
    pub packets_rx: u64,
    pub bytes_tx: u64,
    pub bytes_rx: u64,
    /// PUBLISH packets sent / received (application messages).
    pub messages_tx: u64,
    pub messages_rx: u64,
}

/// A connected client.
#[derive(Debug)]
pub struct Client<T: Transport> {
    io: T,
    version: MqttVersion,
    buf: BytesMut,
    keep_alive: Duration,
    ping_every: Duration,
    ack_timeout: Duration,
    last_tx: Instant,
    ping_sent: Option<Instant>,
    next_id: u16,
    stats: Stats,
    inbox: VecDeque<Message>,
    session_present: bool,
}

impl<T: Transport> Client<T> {
    /// Send CONNECT over `io` and wait for the broker's answer.
    pub fn connect(mut io: T, opts: Options) -> Result<Self> {
        let keep_alive_secs = u16::try_from(opts.keep_alive.as_secs()).unwrap_or(u16::MAX);
        let keep_alive = Duration::from_secs(u64::from(keep_alive_secs));
        let ping_every = opts.ping_interval.unwrap_or(keep_alive / 2);
        let connect = ConnectPacket {
            version: opts.version,
            client_id: opts.client_id,
            clean_start: opts.clean_session,
            keep_alive: keep_alive_secs,
            username: opts.username,
            password: opts.password.map(Bytes::from),
            will: opts.will.map(|w| Will {
                topic: w.topic,
                payload: Bytes::from(w.payload),
                qos: w.qos,
                retain: w.retain,
                properties: Properties::default(),
                delay_interval: 0,
            }),
            properties: Properties::default(),
        };
        let mut client = Self {
            io: {
                io.set_read_timeout(Some(Duration::from_millis(50)))?;
                io
            },
            version: opts.version,
            buf: BytesMut::new(),
            keep_alive,
            ping_every,
            ack_timeout: opts.ack_timeout,
            last_tx: Instant::now(),
            ping_sent: None,
            next_id: 0,
            stats: Stats::default(),
            inbox: VecDeque::new(),
            session_present: false,
        };
        client.send(&Packet::Connect(connect))?;
        let deadline = Instant::now() + client.ack_timeout;
        loop {
            for packet in client.pump(Duration::from_millis(50))? {
                if let Packet::ConnAck(ack) = packet {
                    if !ack.reason_code.is_success() {
                        return Err(Error::Refused(ack.reason_code.0));
                    }
                    client.session_present = ack.session_present;
                    return Ok(client);
                }
            }
            if Instant::now() >= deadline {
                return Err(Error::Timeout("CONNACK"));
            }
        }
    }

    /// Whether the broker kept a session for this client id.
    pub fn session_present(&self) -> bool {
        self.session_present
    }

    pub fn stats(&self) -> Stats {
        self.stats
    }

    /// Publish a message. QoS 1 waits for the broker's PUBACK; QoS 2 is not supported.
    pub fn publish(&mut self, topic: &str, payload: &[u8], qos: QoS, retain: bool) -> Result<()> {
        let packet_id = match qos {
            QoS::AtMostOnce => None,
            QoS::AtLeastOnce => Some(self.next_packet_id()),
            QoS::ExactlyOnce => return Err(Error::Unsupported("publishing at QoS 2")),
        };
        self.send(&Packet::Publish(PublishPacket {
            dup: false,
            qos,
            retain,
            topic: topic.to_string(),
            packet_id,
            payload: Bytes::copy_from_slice(payload),
            properties: Properties::default(),
        }))?;
        self.stats.messages_tx += 1;
        if let Some(id) = packet_id {
            self.wait_for("PUBACK", |p| {
                matches!(p, Packet::PubAck(a) if a.packet_id == id).then_some(())
            })?;
        }
        Ok(())
    }

    /// Subscribe to a topic filter (`+` and `#` allowed) and wait for the SUBACK.
    pub fn subscribe(&mut self, filter: &str, qos: QoS) -> Result<()> {
        let id = self.next_packet_id();
        self.send(&Packet::Subscribe(SubscribePacket {
            packet_id: id,
            filters: vec![SubscribeFilter::new(filter, qos)],
            properties: Properties::default(),
        }))?;
        let code = self.wait_for("SUBACK", |p| match p {
            Packet::SubAck(a) if a.packet_id == id => Some(a.reason_codes.first().copied()),
            _ => None,
        })?;
        match code {
            Some(c) if c.is_success() => Ok(()),
            Some(c) => Err(Error::SubscribeRefused(c.0)),
            None => Err(Error::Unexpected("SUBACK without a reason code")),
        }
    }

    /// Unsubscribe from a topic filter and wait for the UNSUBACK.
    pub fn unsubscribe(&mut self, filter: &str) -> Result<()> {
        let id = self.next_packet_id();
        self.send(&Packet::Unsubscribe(UnsubscribePacket {
            packet_id: id,
            topic_filters: vec![filter.to_string()],
            properties: Properties::default(),
        }))?;
        self.wait_for("UNSUBACK", |p| {
            matches!(p, Packet::UnsubAck(a) if a.packet_id == id).then_some(())
        })
    }

    /// Wait up to `wait` for a message. Also answers the broker's acknowledgements and sends a
    /// PINGREQ when the connection has been idle, so call it at least every few seconds.
    pub fn poll(&mut self, wait: Duration) -> Result<Option<Message>> {
        if let Some(m) = self.inbox.pop_front() {
            return Ok(Some(m));
        }
        self.keep_alive_tick()?;
        self.pump(wait)?;
        self.keep_alive_tick()?;
        Ok(self.inbox.pop_front())
    }

    /// Say goodbye (no will message is sent) and give the transport back.
    pub fn disconnect(mut self) -> Result<T> {
        self.send(&Packet::Disconnect(DisconnectPacket::normal()))?;
        Ok(self.io)
    }

    // ── internals ───────────────────────────────────────────────────────────

    fn next_packet_id(&mut self) -> u16 {
        self.next_id = self.next_id.checked_add(1).unwrap_or(1);
        self.next_id
    }

    fn send(&mut self, packet: &Packet) -> Result<()> {
        let bytes = packet.encode(self.version)?;
        self.io.write_all(&bytes)?;
        self.io.flush()?;
        self.stats.packets_tx += 1;
        self.stats.bytes_tx += bytes.len() as u64;
        self.last_tx = Instant::now();
        Ok(())
    }

    /// Send a PINGREQ when idle, fail when the broker does not answer one.
    fn keep_alive_tick(&mut self) -> Result<()> {
        if self.keep_alive.is_zero() {
            return Ok(());
        }
        if let Some(sent) = self.ping_sent {
            if sent.elapsed() > self.keep_alive {
                return Err(Error::Timeout("PINGRESP"));
            }
        } else if self.last_tx.elapsed() >= self.ping_every {
            self.send(&Packet::PingReq)?;
            self.ping_sent = Some(Instant::now());
        }
        Ok(())
    }

    /// Read what arrives within `wait`, handle protocol packets, queue messages, and return the
    /// acknowledgement packets for the caller that is waiting for one.
    fn pump(&mut self, wait: Duration) -> Result<Vec<Packet>> {
        self.io
            .set_read_timeout(Some(wait.max(Duration::from_millis(1))))?;
        let mut chunk = [0u8; 1024];
        match self.io.read(&mut chunk) {
            Ok(0) => return Err(Error::Closed),
            Ok(n) => {
                self.stats.bytes_rx += n as u64;
                self.buf.extend_from_slice(&chunk[..n]);
            }
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) => {}
            Err(e) => return Err(e.into()),
        }
        let mut acks = Vec::new();
        while let Some(packet) = Packet::decode(&mut self.buf, self.version)? {
            self.stats.packets_rx += 1;
            match packet {
                Packet::Publish(p) => {
                    self.stats.messages_rx += 1;
                    let ack = match (p.qos, p.packet_id) {
                        (QoS::AtLeastOnce, Some(id)) => {
                            Some(Packet::PubAck(SimpleAck::success(id)))
                        }
                        (QoS::ExactlyOnce, Some(id)) => {
                            Some(Packet::PubRec(SimpleAck::success(id)))
                        }
                        _ => None,
                    };
                    self.inbox.push_back(Message {
                        topic: p.topic,
                        payload: p.payload.to_vec(),
                        qos: p.qos,
                        retain: p.retain,
                    });
                    if let Some(a) = ack {
                        self.send(&a)?;
                    }
                }
                Packet::PubRel(a) => {
                    self.send(&Packet::PubComp(SimpleAck::success(a.packet_id)))?
                }
                Packet::PingResp => self.ping_sent = None,
                Packet::Disconnect(_) => return Err(Error::Closed),
                other @ (Packet::ConnAck(_)
                | Packet::PubAck(_)
                | Packet::SubAck(_)
                | Packet::UnsubAck(_)) => {
                    acks.push(other);
                }
                _ => {}
            }
        }
        Ok(acks)
    }

    /// Wait for the first packet `matcher` accepts (messages that arrive meanwhile are queued).
    fn wait_for<R>(
        &mut self,
        what: &'static str,
        mut matcher: impl FnMut(&Packet) -> Option<R>,
    ) -> Result<R> {
        let deadline = Instant::now() + self.ack_timeout;
        loop {
            for packet in self.pump(Duration::from_millis(50))? {
                if let Some(r) = matcher(&packet) {
                    return Ok(r);
                }
            }
            self.keep_alive_tick()?;
            if Instant::now() >= deadline {
                return Err(Error::Timeout(what));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mqtt_proto::connect::{ConnAckPacket, ConnectReasonCode};
    use mqtt_proto::subscribe::{SubAckPacket, SubAckReasonCode};

    /// A scripted transport: `incoming` chunks come out one per read, everything written is kept.
    #[derive(Default)]
    struct Script {
        incoming: VecDeque<Vec<u8>>,
        written: Vec<u8>,
        closed_after_script: bool,
    }

    impl Read for Script {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            match self.incoming.pop_front() {
                Some(chunk) => {
                    let n = chunk.len().min(buf.len());
                    buf[..n].copy_from_slice(&chunk[..n]);
                    if n < chunk.len() {
                        self.incoming.push_front(chunk[n..].to_vec());
                    }
                    Ok(n)
                }
                None if self.closed_after_script => Ok(0),
                None => {
                    std::thread::sleep(Duration::from_millis(2));
                    Err(io::ErrorKind::TimedOut.into())
                }
            }
        }
    }

    impl Write for Script {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.written.extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl Transport for Script {
        fn set_read_timeout(&mut self, _: Option<Duration>) -> io::Result<()> {
            Ok(())
        }
    }

    fn bytes(p: &Packet) -> Vec<u8> {
        p.encode(MqttVersion::V311).expect("encodes").to_vec()
    }

    fn connack(code: u8) -> Vec<u8> {
        bytes(&Packet::ConnAck(ConnAckPacket {
            session_present: false,
            reason_code: ConnectReasonCode::from_v311_return_code(code),
            properties: Properties::default(),
        }))
    }

    fn publish_in(topic: &str, payload: &[u8], qos: QoS, id: Option<u16>) -> Vec<u8> {
        bytes(&Packet::Publish(PublishPacket {
            dup: false,
            qos,
            retain: false,
            topic: topic.into(),
            packet_id: id,
            payload: Bytes::copy_from_slice(payload),
            properties: Properties::default(),
        }))
    }

    fn connected(extra: Vec<Vec<u8>>) -> Client<Script> {
        let mut s = Script::default();
        s.incoming.push_back(connack(0));
        s.incoming.extend(extra);
        Client::connect(s, Options::new("test")).expect("connects")
    }

    fn decode_all(written: &[u8]) -> Vec<Packet> {
        let mut buf = BytesMut::from(written);
        let mut out = Vec::new();
        while let Some(p) = Packet::decode(&mut buf, MqttVersion::V311).expect("decodes") {
            out.push(p);
        }
        out
    }

    #[test]
    fn connect_sends_the_options_and_accepts_a_connack() {
        let mut s = Script::default();
        s.incoming.push_back(connack(0));
        let opts = Options::new("c6").credentials("u", "p").will(WillMessage {
            topic: "dev/status".into(),
            payload: b"offline".to_vec(),
            qos: QoS::AtLeastOnce,
            retain: true,
        });
        let client = Client::connect(s, opts).expect("connects");
        let sent = decode_all(&client.io.written);
        let Packet::Connect(c) = &sent[0] else {
            panic!("first packet is CONNECT")
        };
        assert_eq!(
            (c.client_id.as_str(), c.keep_alive, c.clean_start),
            ("c6", 30, true)
        );
        assert_eq!(c.username.as_deref(), Some("u"));
        assert_eq!(
            c.will.as_ref().map(|w| w.topic.as_str()),
            Some("dev/status")
        );
        assert_eq!(client.stats().packets_tx, 1);
        assert_eq!(client.stats().packets_rx, 1);
    }

    #[test]
    fn a_refused_connection_says_why() {
        let mut s = Script::default();
        s.incoming.push_back(connack(5)); // not authorised
        assert!(matches!(
            Client::connect(s, Options::new("x")),
            Err(Error::Refused(_))
        ));
    }

    #[test]
    fn no_connack_times_out() {
        let opts = Options {
            ack_timeout: Duration::from_millis(100),
            ..Options::new("x")
        };
        assert!(matches!(
            Client::connect(Script::default(), opts),
            Err(Error::Timeout("CONNACK"))
        ));
    }

    #[test]
    fn qos0_publish_is_one_packet_and_qos2_is_refused() {
        let mut c = connected(vec![]);
        c.publish("a/b", b"hi", QoS::AtMostOnce, true)
            .expect("sent");
        let sent = decode_all(&c.io.written);
        let Packet::Publish(p) = &sent[1] else {
            panic!("a PUBLISH")
        };
        assert_eq!(
            (p.topic.as_str(), &p.payload[..], p.retain, p.packet_id),
            ("a/b", &b"hi"[..], true, None)
        );
        assert_eq!(c.stats().messages_tx, 1);
        assert!(matches!(
            c.publish("a", b"x", QoS::ExactlyOnce, false),
            Err(Error::Unsupported(_))
        ));
    }

    #[test]
    fn qos1_publish_waits_for_its_puback() {
        let puback = bytes(&Packet::PubAck(SimpleAck::success(1)));
        let mut c = connected(vec![puback]);
        c.publish("a", b"x", QoS::AtLeastOnce, false)
            .expect("acknowledged");
        // without an answer it times out
        let mut c2 = connected(vec![]);
        c2.ack_timeout = Duration::from_millis(80);
        assert!(matches!(
            c2.publish("a", b"x", QoS::AtLeastOnce, false),
            Err(Error::Timeout("PUBACK"))
        ));
    }

    #[test]
    fn subscribe_checks_the_suback_and_keeps_messages_that_arrive_first() {
        let early = publish_in("t/early", b"1", QoS::AtMostOnce, None);
        let suback = bytes(&Packet::SubAck(SubAckPacket {
            packet_id: 1,
            reason_codes: vec![SubAckReasonCode::granted(QoS::AtLeastOnce)],
            properties: Properties::default(),
        }));
        let mut c = connected(vec![early, suback]);
        c.subscribe("t/#", QoS::AtLeastOnce).expect("subscribed");
        let m = c
            .poll(Duration::from_millis(10))
            .expect("polls")
            .expect("the early message");
        assert_eq!(
            (m.topic.as_str(), m.payload.as_slice()),
            ("t/early", &b"1"[..])
        );
    }

    #[test]
    fn a_refused_subscription_is_an_error() {
        let suback = bytes(&Packet::SubAck(SubAckPacket {
            packet_id: 1,
            reason_codes: vec![SubAckReasonCode(0x80)],
            properties: Properties::default(),
        }));
        let mut c = connected(vec![suback]);
        assert!(matches!(
            c.subscribe("t", QoS::AtMostOnce),
            Err(Error::SubscribeRefused(0x80))
        ));
    }

    #[test]
    fn incoming_qos1_is_acknowledged_and_qos2_is_completed() {
        let mut c = connected(vec![
            publish_in("a", b"one", QoS::AtLeastOnce, Some(7)),
            publish_in("b", b"two", QoS::ExactlyOnce, Some(8)),
            bytes(&Packet::PubRel(SimpleAck::success(8))),
        ]);
        let first = c
            .poll(Duration::from_millis(10))
            .expect("polls")
            .expect("message");
        assert_eq!(first.topic, "a");
        let second = c
            .poll(Duration::from_millis(10))
            .expect("polls")
            .expect("message");
        assert_eq!(second.topic, "b");
        c.poll(Duration::from_millis(10)).expect("polls");
        let sent = decode_all(&c.io.written);
        assert!(
            sent.iter()
                .any(|p| matches!(p, Packet::PubAck(a) if a.packet_id == 7))
        );
        assert!(
            sent.iter()
                .any(|p| matches!(p, Packet::PubRec(a) if a.packet_id == 8))
        );
        assert!(
            sent.iter()
                .any(|p| matches!(p, Packet::PubComp(a) if a.packet_id == 8))
        );
        assert_eq!(c.stats().messages_rx, 2);
    }

    #[test]
    fn a_packet_split_over_several_reads_is_assembled() {
        let whole = publish_in("split/topic", b"payload", QoS::AtMostOnce, None);
        let (a, b) = whole.split_at(5);
        let (b1, b2) = b.split_at(3);
        let mut c = connected(vec![a.to_vec(), b1.to_vec(), b2.to_vec()]);
        let mut got = None;
        for _ in 0..5 {
            if let Some(m) = c.poll(Duration::from_millis(5)).expect("polls") {
                got = Some(m);
                break;
            }
        }
        assert_eq!(got.expect("assembled").topic, "split/topic");
    }

    #[test]
    fn an_idle_connection_is_pinged_and_a_silent_broker_is_a_timeout() {
        let opts = Options {
            keep_alive: Duration::from_secs(1),
            ping_interval: Some(Duration::from_millis(20)),
            ..Options::new("x")
        };
        let mut s = Script::default();
        s.incoming.push_back(connack(0));
        let mut c = Client::connect(s, opts).expect("connects");
        std::thread::sleep(Duration::from_millis(40));
        c.poll(Duration::from_millis(5)).expect("polls");
        assert!(
            decode_all(&c.io.written)
                .iter()
                .any(|p| matches!(p, Packet::PingReq)),
            "a PINGREQ went out"
        );
        // the broker never answers: after the keep-alive the connection is declared dead
        std::thread::sleep(Duration::from_millis(1050));
        assert!(matches!(
            c.poll(Duration::from_millis(5)),
            Err(Error::Timeout("PINGRESP"))
        ));
    }

    #[test]
    fn a_pingresp_clears_the_wait() {
        let opts = Options {
            keep_alive: Duration::from_secs(1),
            ping_interval: Some(Duration::from_millis(20)),
            ..Options::new("x")
        };
        let mut s = Script::default();
        s.incoming.push_back(connack(0));
        let mut c = Client::connect(s, opts).expect("connects");
        std::thread::sleep(Duration::from_millis(40));
        c.poll(Duration::from_millis(5)).expect("pings");
        c.io.incoming.push_back(bytes(&Packet::PingResp));
        c.poll(Duration::from_millis(5)).expect("pong read");
        assert!(c.ping_sent.is_none());
    }

    #[test]
    fn a_closed_connection_is_reported() {
        let mut c = connected(vec![]);
        c.io.closed_after_script = true;
        assert!(matches!(
            c.poll(Duration::from_millis(5)),
            Err(Error::Closed)
        ));
    }

    #[test]
    fn a_server_disconnect_is_reported() {
        let mut c = connected(vec![bytes(&Packet::Disconnect(DisconnectPacket::normal()))]);
        assert!(matches!(
            c.poll(Duration::from_millis(5)),
            Err(Error::Closed)
        ));
    }

    #[test]
    fn garbage_from_the_broker_is_a_protocol_error() {
        let mut c = connected(vec![vec![0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]]);
        assert!(matches!(
            c.poll(Duration::from_millis(5)),
            Err(Error::Protocol(_))
        ));
    }

    #[test]
    fn disconnect_sends_the_goodbye_and_returns_the_transport() {
        let c = connected(vec![]);
        let io = c.disconnect().expect("sent");
        assert!(
            decode_all(&io.written)
                .iter()
                .any(|p| matches!(p, Packet::Disconnect(_)))
        );
    }

    #[test]
    fn packet_ids_skip_zero_when_they_wrap() {
        let mut c = connected(vec![]);
        c.next_id = u16::MAX;
        assert_eq!(c.next_packet_id(), 1);
    }
}
