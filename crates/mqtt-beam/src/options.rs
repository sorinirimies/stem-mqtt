//! Decoding client options and broker config from Erlang maps.
//!
//! Defaults stay in Rust (`ConnectOptions::new`, `MqttBrokerConfig::new`); a
//! map only overrides what it names. Unknown keys raise, so a typo is not
//! silently ignored.

use mqtt_broker::{BrokerTlsConfig, MqttBrokerConfig};
use mqtt_client::{ConnectOptions, MqttVersion, QoS, TlsOptions, WillOptions};
use rustler::types::map::MapIterator;
use rustler::{Binary, Decoder, NifResult, Term};

use crate::{argument_error, atoms};

/// The entries of `map` as `(key name, value)`, keys being atoms.
fn entries<'a>(map: Term<'a>, what: &str) -> NifResult<Vec<(String, Term<'a>)>> {
    let iter =
        MapIterator::new(map).ok_or_else(|| argument_error(format!("{what} must be a map")))?;
    iter.map(|(key, value)| {
        key.atom_to_string()
            .map(|name| (name, value))
            .map_err(|_| argument_error(format!("{what} keys must be atoms")))
    })
    .collect()
}

fn field<'a, T: Decoder<'a>>(
    value: Term<'a>,
    what: &str,
    key: &str,
    expected: &str,
) -> NifResult<T> {
    value
        .decode()
        .map_err(|_| argument_error(format!("{what}.{key} must be {expected}")))
}

/// An unsigned integer of type `T`. Decoded as `u64` and narrowed: rustler's
/// `u16` decoder truncates (70000 would become port 4464) instead of failing.
fn uint<T: TryFrom<u64>>(value: Term, what: &str, key: &str, expected: &str) -> NifResult<T> {
    let wide: u64 = field(value, what, key, expected)?;
    T::try_from(wide).map_err(|_| argument_error(format!("{what}.{key} must be {expected}")))
}

fn bytes(value: Term, what: &str, key: &str) -> NifResult<Vec<u8>> {
    let binary: Binary = field(value, what, key, "a binary")?;
    Ok(binary.as_slice().to_vec())
}

fn optional_bytes(value: Term, what: &str, key: &str) -> NifResult<Option<Vec<u8>>> {
    if is_nil(value) {
        Ok(None)
    } else {
        bytes(value, what, key).map(Some)
    }
}

fn is_nil(value: Term) -> bool {
    value.atom_to_string().is_ok_and(|name| name == "nil")
}

fn unknown(what: &str, key: &str) -> rustler::Error {
    argument_error(format!("unknown {what} option :{key}"))
}

/// QoS as the integer 0, 1 or 2, the way MQTT and Elixir MQTT libraries write it.
pub(crate) fn qos(value: Term) -> NifResult<QoS> {
    match value.decode::<u8>() {
        Ok(0) => Ok(QoS::AtMostOnce),
        Ok(1) => Ok(QoS::AtLeastOnce),
        Ok(2) => Ok(QoS::ExactlyOnce),
        _ => Err(argument_error("qos must be 0, 1 or 2")),
    }
}

fn version(value: Term) -> NifResult<MqttVersion> {
    let atom: rustler::Atom = value
        .decode()
        .map_err(|_| argument_error("version must be :v5 or :v311"))?;
    if atom == atoms::v5() {
        Ok(MqttVersion::V5)
    } else if atom == atoms::v311() {
        Ok(MqttVersion::V311)
    } else {
        Err(argument_error("version must be :v5 or :v311"))
    }
}

fn will(value: Term) -> NifResult<Option<WillOptions>> {
    if is_nil(value) {
        return Ok(None);
    }
    let what = "will";
    let mut topic = None;
    let mut out = WillOptions {
        topic: String::new(),
        payload: Vec::new(),
        qos: QoS::AtMostOnce,
        retain: false,
    };
    for (key, value) in entries(value, what)? {
        match key.as_str() {
            "topic" => topic = Some(field(value, what, &key, "a string")?),
            "payload" => out.payload = bytes(value, what, &key)?,
            "qos" => out.qos = qos(value)?,
            "retain" => out.retain = field(value, what, &key, "a boolean")?,
            _ => return Err(unknown(what, &key)),
        }
    }
    out.topic = topic.ok_or_else(|| argument_error("will.topic is required"))?;
    Ok(Some(out))
}

fn client_tls(value: Term) -> NifResult<Option<TlsOptions>> {
    if is_nil(value) {
        return Ok(None);
    }
    let what = "tls";
    let mut out = TlsOptions::new();
    for (key, value) in entries(value, what)? {
        match key.as_str() {
            "ca_cert_pem" => out.ca_cert_pem = optional_bytes(value, what, &key)?,
            "client_cert_pem" => out.client_cert_pem = optional_bytes(value, what, &key)?,
            "client_key_pem" => out.client_key_pem = optional_bytes(value, what, &key)?,
            "insecure_skip_certificate_verification" => {
                out.insecure_skip_certificate_verification = field(value, what, &key, "a boolean")?
            }
            _ => return Err(unknown(what, &key)),
        }
    }
    Ok(Some(out))
}

/// `ConnectOptions` from a map; `:host` is required, everything else defaults
/// as in `ConnectOptions::new` (port 1883, MQTT 5.0, an empty client id).
pub(crate) fn connect_options(map: Term) -> NifResult<ConnectOptions> {
    let what = "client";
    let mut host = None;
    let mut o = ConnectOptions::new("", 1883, "", MqttVersion::V5);
    for (key, value) in entries(map, what)? {
        let k = key.as_str();
        match k {
            "host" => host = Some(field(value, what, k, "a string")?),
            "port" => o.port = uint(value, what, k, "a port number")?,
            "client_id" => o.client_id = field(value, what, k, "a string")?,
            "version" => o.version = version(value)?,
            "clean_start" => o.clean_start = field(value, what, k, "a boolean")?,
            "keep_alive_secs" => o.keep_alive_secs = uint(value, what, k, "0..65535")?,
            "username" => o.username = field(value, what, k, "a string or nil")?,
            "password" => o.password = optional_bytes(value, what, k)?,
            "will" => o.will = will(value)?,
            "connect_timeout_secs" => {
                o.connect_timeout_secs = uint(value, what, k, "a non-negative integer")?
            }
            "operation_timeout_secs" => {
                o.operation_timeout_secs = uint(value, what, k, "a non-negative integer")?
            }
            "auto_reconnect" => o.auto_reconnect = field(value, what, k, "a boolean")?,
            "reconnect_backoff_secs" => {
                o.reconnect_backoff_secs = uint(value, what, k, "a non-negative integer")?
            }
            "reconnect_max_backoff_secs" => {
                o.reconnect_max_backoff_secs = uint(value, what, k, "a non-negative integer")?
            }
            "tls" => o.tls = client_tls(value)?,
            "max_packet_size" => {
                o.max_packet_size = uint(value, what, k, "a non-negative integer")?
            }
            _ => return Err(unknown(what, k)),
        }
    }
    o.host = host.ok_or_else(|| argument_error("client option :host is required"))?;
    Ok(o)
}

fn broker_qos(value: Term) -> NifResult<mqtt_broker::QoS> {
    qos(value).map(Into::into)
}

fn broker_tls(value: Term) -> NifResult<Option<BrokerTlsConfig>> {
    if is_nil(value) {
        return Ok(None);
    }
    let what = "tls";
    let (mut port, mut cert_pem, mut key_pem, mut client_ca_pem) = (None, None, None, None);
    for (key, value) in entries(value, what)? {
        match key.as_str() {
            "port" => port = Some(uint(value, what, &key, "a port number")?),
            "cert_pem" => cert_pem = Some(bytes(value, what, &key)?),
            "key_pem" => key_pem = Some(bytes(value, what, &key)?),
            "client_ca_pem" => client_ca_pem = optional_bytes(value, what, &key)?,
            _ => return Err(unknown(what, &key)),
        }
    }
    let required = |name: &str| argument_error(format!("tls.{name} is required"));
    Ok(Some(BrokerTlsConfig {
        port: port.ok_or_else(|| required("port"))?,
        cert_pem: cert_pem.ok_or_else(|| required("cert_pem"))?,
        key_pem: key_pem.ok_or_else(|| required("key_pem"))?,
        client_ca_pem,
    }))
}

/// `MqttBrokerConfig` from a map. Unlike the standalone broker, it binds to
/// 127.0.0.1 unless `:bind_address` says otherwise; port 1883 by default,
/// `0` picks a free port (see `bound_port`).
pub(crate) fn broker_config(map: Term) -> NifResult<MqttBrokerConfig> {
    let what = "broker";
    let mut c = MqttBrokerConfig::new("127.0.0.1", 1883);
    for (key, value) in entries(map, what)? {
        let k = key.as_str();
        match k {
            "bind_address" => c.bind_address = field(value, what, k, "a string")?,
            "port" => c.port = uint(value, what, k, "a port number")?,
            "ws_port" => {
                c.ws_port = if is_nil(value) {
                    None
                } else {
                    Some(uint(value, what, k, "a port number or nil")?)
                }
            }
            "allow_anonymous" => c.allow_anonymous = field(value, what, k, "a boolean")?,
            "max_clients" => c.max_clients = uint(value, what, k, "a non-negative integer")?,
            "max_qos" => c.max_qos = broker_qos(value)?,
            "max_retained_messages" => {
                c.max_retained_messages = uint(value, what, k, "a non-negative integer")?
            }
            "max_queued_per_client" => {
                c.max_queued_per_client = uint(value, what, k, "a non-negative integer")?
            }
            "redelivery_interval_secs" => {
                c.redelivery_interval_secs = uint(value, what, k, "a non-negative integer")?
            }
            "tls" => c.tls = broker_tls(value)?,
            "max_packet_size" => {
                c.max_packet_size = uint(value, what, k, "a non-negative integer")?
            }
            "max_outbound_queue" => {
                c.max_outbound_queue = uint(value, what, k, "a non-negative integer")?
            }
            "session_expiry_secs" => {
                c.session_expiry_secs = uint(value, what, k, "a non-negative integer")?
            }
            _ => return Err(unknown(what, k)),
        }
    }
    Ok(c)
}
