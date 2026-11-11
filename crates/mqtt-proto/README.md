# stem-mqtt-proto

The **MQTT 3.1.1 / 5.0 wire protocol** as a pure, synchronous codec: no async runtime, no TLS, no
sockets, no global state. It is the codec inside [`stem-mqtt-client`](../mqtt-client) and
[`stem-mqtt-broker`](../mqtt-broker), and it is small enough to run on a microcontroller that
brings its own transport (an ESP32 over `std::net::TcpStream`, for example).

| | |
|---|---|
| Packets | CONNECT, CONNACK, PUBLISH, PUBACK/PUBREC/PUBREL/PUBCOMP, SUBSCRIBE, SUBACK, UNSUBSCRIBE, UNSUBACK, PINGREQ/PINGRESP, DISCONNECT, AUTH |
| Versions | MQTT 3.1.1 and MQTT 5.0 (properties, reason codes), chosen per connection |
| Topics | topic name and filter validation, `+` / `#` matching |
| Dependencies | `bytes`, `thiserror` |
| Safety | no `unsafe`; fuzzed (decoder, round trips, chunked input); the decoder never panics, loops or over-allocates on untrusted bytes |

```toml
[dependencies]
stem-mqtt-proto = "0.8"
```

```rust
use bytes::Bytes;
use mqtt_proto::packet::Packet;
use mqtt_proto::properties::Properties;
use mqtt_proto::publish::PublishPacket;
use mqtt_proto::{MqttError, MqttVersion, QoS};

// Encode a PUBLISH ...
let publish = Packet::Publish(PublishPacket {
    dup: false,
    qos: QoS::AtLeastOnce,
    retain: false,
    topic: "home/living-room/light/set".into(),
    packet_id: Some(1),
    payload: Bytes::from_static(b"ON"),
    properties: Properties::default(),
});
let mut wire = publish.encode(MqttVersion::V311)?;

// ... and decode whatever arrives (`None` until a whole packet is buffered).
let decoded = Packet::decode(&mut wire, MqttVersion::V311)?;
assert_eq!(decoded, Some(publish));
# Ok::<(), MqttError>(())
```

The client crate re-exports this crate as `mqtt_client::protocol` (and `mqtt_client::error`), so
existing code keeps compiling, and declares `QoS`, `MqttVersion` and `MqttError` to UniFFI as remote
types: the generated bindings for Kotlin, Swift, Python and the other languages are unchanged.

## License

MIT
