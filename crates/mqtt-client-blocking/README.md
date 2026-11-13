# stem-mqtt-client-blocking

The **blocking** MQTT client of the stem-mqtt family: the small, runtime-free counterpart of the async
[`stem-mqtt-client`](../mqtt-client). A MQTT 3.1.1 / 5.0 client on top of [`stem-mqtt-proto`](../mqtt-proto). No async
runtime and no TLS stack of its own (an optional `rustls` feature adds one for PCs): it drives the codec over any `std::io` transport, so it runs on
a PC, in a test, and on an **ESP32 with ESP-IDF `std`** (where `TcpStream` is a lwIP socket).

```rust,no_run
use std::net::TcpStream;
use std::time::Duration;
use mqtt_client_blocking::{Client, Options, QoS};

let stream = TcpStream::connect("192.168.1.10:1883")?;
let mut client = Client::connect(stream, Options::new("weather-c6"))?;
client.subscribe("home/+/command", QoS::AtLeastOnce)?;
client.publish("home/c6/status", b"online", QoS::AtLeastOnce, true)?;
loop {
    if let Some(msg) = client.poll(Duration::from_millis(500))? {
        println!("{}: {}", msg.topic, String::from_utf8_lossy(&msg.payload));
    }
}
# Ok::<(), mqtt_client_blocking::Error>(())
```

You own the loop: call `poll` regularly. It receives messages, acknowledges them, and sends a
PINGREQ when the connection is idle (a silent broker becomes `Error::Timeout`). Reconnecting is
yours too: after an error, drop the client and connect again (with a growing pause).

| | |
|---|---|
| Publish | QoS 0 and 1 (QoS 1 waits for the PUBACK); retained messages; QoS 2 is refused with `Error::Unsupported` |
| Receive | QoS 0, 1 and 2 (a QoS 2 message is delivered once and completed with PUBREC/PUBCOMP) |
| Session | credentials, will message, clean session, keep-alive, MQTT 3.1.1 or 5.0 |
| Transport | anything implementing `Transport` (`Read + Write + set_read_timeout`); `TcpStream` out of the box; TLS via the `rustls` feature (PCs) or your SDK's TLS stream (firmware), see [Securing a client](#securing-a-client) |
| Stats | `Client::stats()`: packets, bytes and messages each way, for a status screen |

| | `stem-mqtt-client` | `stem-mqtt-client-blocking` (this crate) |
|---|---|---|
| Style | async (tokio), automatic reconnect, TLS built in, UniFFI bindings | blocking, you run the loop and reconnect, you bring the transport |
| Use it for | apps, servers, phones | firmware (ESP32 `std`), small tools, tests |

Both speak through [`stem-mqtt-proto`](../mqtt-proto) and talk to any broker, such as [`stem-mqtt-broker`](../mqtt-broker).

## On an ESP32

Checked on an ESP32-C6 (no PSRAM, about 150 KB of heap left after a Slint UI) against `stem-mqtt-broker`:

* The client needs only a few KB: a 1 KB read buffer, the packets, and the socket. Run it **in your
  main loop** (`poll` with a 1 ms timeout) rather than in a thread when memory is tight: a thread's
  stack was the difference between a working HTTPS fetch and an out-of-memory abort.
* A blocking `connect` can stall the loop; use `TcpStream::connect_timeout` and a backoff.
* Close the connection cleanly (`disconnect`) before something that needs the memory (an HTTPS
  request); a clean goodbye does not trigger the will.

## Securing a client

The client speaks MQTT over any `Transport`, so TLS is a matter of handing it a TLS stream. Use the
broker's TLS port (8883 by convention) and **verify the broker's certificate**: a client that does not
check it is encrypted but can still be impersonated.

### PC, Linux gateway, tests: feature `rustls`

```toml
stem-mqtt-client-blocking = { version = "0.8", features = ["rustls"] }
```

```rust,ignore
// (compiled as the `tls` module's doc test, which needs the feature)
use mqtt_client_blocking::{Client, Options, tls};

let ca = std::fs::read("ca.pem")?; // your private CA, or the broker's self-signed certificate
let config = tls::client_config(Some(&ca), None)?; // None: the public web roots instead
let stream = tls::connect("broker.local", 8883, config)?; // verifies the certificate for "broker.local"
let client = Client::connect(stream, Options::new("gateway"))?;
# drop(client);
# Ok::<(), Box<dyn std::error::Error>>(())
```

* **Mutual TLS:** pass `Some(tls::ClientIdentity { cert_pem, key_pem })` as the second argument.
* The handshake runs inside `Client::connect`; a rejected certificate is an `Error::Io` there.
* The read timeout of `poll` is set on the TCP socket and is safe in the middle of a TLS record.
* Try it against a local TLS broker (it writes a self-signed certificate to `<tmp>/stem-mqtt-tls/ca.pem`):

  ```text
  cargo run -p stem-mqtt-broker --example tls_broker
  cargo run -p stem-mqtt-client-blocking --features rustls --example mqtts -- sub localhost 8883 'home/#' "${TMPDIR:-/tmp}/stem-mqtt-tls/ca.pem"
  cargo run -p stem-mqtt-client-blocking --features rustls --example mqtts -- pub localhost 8883 home/light on "${TMPDIR:-/tmp}/stem-mqtt-tls/ca.pem"
  ```

  Without the CA argument the same commands fail with `InvalidCertificate(UnknownIssuer)`; the
  [TLS demo](https://github.com/sorinirimies/stem-mqtt/blob/main/examples/vhs/tls-demo.tape) (`just vhs-tape tls-demo`) records exactly that.

  ![TLS demo](https://raw.githubusercontent.com/sorinirimies/stem-mqtt/main/examples/vhs/generated/tls-demo.gif)

### Firmware (ESP32): the ESP-IDF TLS stack

Do not use rustls on the ESP32: `ring` does not build cleanly for every chip and costs flash and RAM,
while ESP-IDF already ships mbedtls with a certificate bundle in `esp-tls`. Wrap `esp-idf-svc`'s `EspTls`
in a newtype that implements `Transport` (the trait is this crate's, the stream is not, so a newtype is
required). **This sketch is not compiled in this repository** (it needs the ESP toolchain); check it
against your `esp-idf-svc` version and test the read timeout on the device:

```rust,ignore
use std::io::{self, Read, Write};
use std::time::Duration;
use esp_idf_svc::sys;
use esp_idf_svc::tls::{Config, EspTls, InternalSocket, X509};
use mqtt_client_blocking::Transport;

pub struct EspTlsTransport {
    tls: EspTls<InternalSocket>,
    fd: i32,
}

impl EspTlsTransport {
    /// `ca_pem` must end with a NUL byte: `concat!(include_str!("ca.pem"), "\0").as_bytes()`.
    pub fn connect(host: &str, port: u16, ca_pem: &'static [u8]) -> io::Result<Self> {
        let mut tls = EspTls::new().map_err(io::Error::other)?;
        let config = Config {
            ca_cert: Some(X509::pem_until_nul(ca_pem)), // or the bundle: leave None, use_crt_bundle_attach = true
            common_name: Some(host),                    // the name checked against the certificate
            timeout_ms: 10_000,                         // connect + handshake
            ..Config::new()
        };
        tls.connect(host, port, &config).map_err(io::Error::other)?;
        let mut fd = -1;
        // SAFETY: `tls.context_handle()` is the live esp_tls handle owned by `tls`.
        unsafe { sys::esp!(sys::esp_tls_get_conn_sockfd(tls.context_handle(), &mut fd)) }
            .map_err(io::Error::other)?;
        Ok(Self { tls, fd })
    }
}

impl Read for EspTlsTransport {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.tls.read(buf).map_err(|e| {
            // With SO_RCVTIMEO set, a quiet socket surfaces as WANT_READ: that is a timeout, not a failure.
            if e.code() == sys::ESP_TLS_ERR_SSL_WANT_READ as i32 {
                io::ErrorKind::WouldBlock.into()
            } else {
                io::Error::other(e)
            }
        })
    }
}

impl Write for EspTlsTransport {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.tls.write(buf).map_err(io::Error::other)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Transport for EspTlsTransport {
    fn set_read_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        let t = timeout.unwrap_or_default(); // zero: block
        let tv = sys::timeval { tv_sec: t.as_secs() as _, tv_usec: t.subsec_micros() as _ };
        // SAFETY: `fd` is the socket of the live TLS connection; `tv` outlives the call.
        let rc = unsafe {
            sys::lwip_setsockopt(self.fd, sys::SOL_SOCKET as _, sys::SO_RCVTIMEO as _,
                &tv as *const _ as *const _, core::mem::size_of::<sys::timeval>() as _)
        };
        if rc == 0 { Ok(()) } else { Err(io::Error::last_os_error()) }
    }
}
```

Things to check on the device: the read timeout (a timeout in `poll` must come back as `WouldBlock`, not
as an error that ends the session), heap use during the handshake (about 40 KB of mbedtls buffers; on a
chip without PSRAM connect **before** you bring up a UI, and keep the session open instead of reconnecting
often), and the clock: certificate validity needs SNTP time, or the handshake fails with a date error.

## Example

```text
cargo run -p stem-mqtt-client-blocking --example mqtt -- sub 127.0.0.1:1883 'home/#'
cargo run -p stem-mqtt-client-blocking --example mqtt -- pub 127.0.0.1:1883 home/kitchen/light on --retain
```

Over TLS: the `mqtts` example, see [Securing a client](#securing-a-client).

## Tests

Unit tests use a scripted in-memory transport (exact bytes, packets split across reads, timeouts,
refusals, keep-alive). The integration tests start a real `stem-mqtt-broker` on a loopback port and
check: publish/subscribe at QoS 0 and 1, retained messages, the will on an abrupt drop (and none on a
clean disconnect), credentials, keep-alive across an idle period, a 150 KB payload, a broker that goes
away, and interop with the async `stem-mqtt-client`. With `--features rustls`, `tests/tls.rs` runs the client
against the broker's TLS listener: a private CA, a 150 KB payload across TLS records, an untrusted
certificate and a certificate for another host (both refused), mutual TLS with and without a client
certificate, and bad PEM input.

## License

MIT
