# stem-mqtt-sync

A small **blocking** MQTT 3.1.1 / 5.0 client on top of [`stem-mqtt-proto`](../mqtt-proto). No async
runtime and no TLS stack of its own: it drives the codec over any `std::io` transport, so it runs on
a PC, in a test, and on an **ESP32 with ESP-IDF `std`** (where `TcpStream` is a lwIP socket).

```rust,no_run
use std::net::TcpStream;
use std::time::Duration;
use mqtt_sync::{Client, Options, QoS};

let stream = TcpStream::connect("192.168.1.10:1883")?;
let mut client = Client::connect(stream, Options::new("weather-c6"))?;
client.subscribe("home/+/command", QoS::AtLeastOnce)?;
client.publish("home/c6/status", b"online", QoS::AtLeastOnce, true)?;
loop {
    if let Some(msg) = client.poll(Duration::from_millis(500))? {
        println!("{}: {}", msg.topic, String::from_utf8_lossy(&msg.payload));
    }
}
# Ok::<(), mqtt_sync::Error>(())
```

You own the loop: call `poll` regularly. It receives messages, acknowledges them, and sends a
PINGREQ when the connection is idle (a silent broker becomes `Error::Timeout`). Reconnecting is
yours too: after an error, drop the client and connect again (with a growing pause).

| | |
|---|---|
| Publish | QoS 0 and 1 (QoS 1 waits for the PUBACK); retained messages; QoS 2 is refused with `Error::Unsupported` |
| Receive | QoS 0, 1 and 2 (a QoS 2 message is delivered once and completed with PUBREC/PUBCOMP) |
| Session | credentials, will message, clean session, keep-alive, MQTT 3.1.1 or 5.0 |
| Transport | anything implementing `Transport` (`Read + Write + set_read_timeout`); `TcpStream` out of the box, a TLS stream is one small impl |
| Stats | `Client::stats()`: packets, bytes and messages each way, for a status screen |

## On an ESP32

Checked on an ESP32-C6 (no PSRAM, about 150 KB of heap left after a Slint UI) against `stem-mqtt-broker`:

* The client needs only a few KB: a 1 KB read buffer, the packets, and the socket. Run it **in your
  main loop** (`poll` with a 1 ms timeout) rather than in a thread when memory is tight: a thread's
  stack was the difference between a working HTTPS fetch and an out-of-memory abort.
* A blocking `connect` can stall the loop; use `TcpStream::connect_timeout` and a backoff.
* Close the connection cleanly (`disconnect`) before something that needs the memory (an HTTPS
  request); a clean goodbye does not trigger the will.

## Example

```text
cargo run -p stem-mqtt-sync --example mqtt -- sub 127.0.0.1:1883 'home/#'
cargo run -p stem-mqtt-sync --example mqtt -- pub 127.0.0.1:1883 home/kitchen/light on --retain
```

## Tests

Unit tests use a scripted in-memory transport (exact bytes, packets split across reads, timeouts,
refusals, keep-alive). The integration tests start a real `stem-mqtt-broker` on a loopback port and
check: publish/subscribe at QoS 0 and 1, retained messages, the will on an abrupt drop (and none on a
clean disconnect), credentials, keep-alive across an idle period, a 150 KB payload, a broker that goes
away, and interop with the async `stem-mqtt-client`.

## License

MIT
