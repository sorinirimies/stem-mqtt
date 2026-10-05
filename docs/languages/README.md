# stem-mqtt language guides

stem-mqtt is one Rust implementation (an MQTT 3.1.1 / 5.0 **client** and **broker**) exposed to ten
languages. Every guide below follows the same outline — install, requirements, quick start, client,
broker, authentication, TLS, language notes, building from source, troubleshooting — so you can move
between them. This page holds what is common to all of them.

| Language | Guide | Package | How it is generated | Callbacks |
| --- | --- | --- | --- | :-: |
| Rust | [rust.md](rust.md) | `stem-mqtt-client`, `stem-mqtt-broker` (crates.io) | native | ✅ |
| Python | [python.md](python.md) | `stem-mqtt-client`, `stem-mqtt-broker` (PyPI) | UniFFI (built in) | ✅ |
| Node.js | [node.md](node.md) | `@sorinirimies/stem-mqtt-node` (npm) | uniffi-bindgen-node-js | ✅ |
| Kotlin | [kotlin.md](kotlin.md) | `stem-mqtt-kotlin` / `stem-mqtt-android` (Maven) | UniFFI (built in) | ✅ |
| Java | [java.md](java.md) | `stem-mqtt-java` (Maven) | uniffi-bindgen-java | ✅ |
| C# | [csharp.md](csharp.md) | `StemMqtt` (NuGet) | uniffi-bindgen-cs | ✅ |
| Go | [go.md](go.md) | `github.com/<owner>/stem-mqtt-go` | uniffi-bindgen-go | ✅ |
| Swift | [swift.md](swift.md) | `StemMqttSwift-<version>.zip` | UniFFI (built in) | ✅ |
| Dart | [dart.md](dart.md) | `stem_mqtt` (pub.dev) | uniffi-dart | ❌ pull-style |
| Haskell | [haskell.md](haskell.md) | `stem-mqtt` (Hackage) | uniffi-bindgen-haskell | ❌ pull-style |

Ruby is not supported ([why](../../packaging/ruby/README.md)).

## The model

Each language gets **two components**, a client and a broker, which are separate modules/packages:

- **`MqttClient`** — built from a `ConnectOptions` record. `connect`, `subscribe`, `unsubscribe`,
  `publish`, `disconnect`, `is_connected`.
- **`MqttBroker`** — built from an `MqttBrokerConfig` record. `start`, `stop`, `is_running`,
  `bound_port`, `bound_ws_port`, `bound_tls_port`, `client_count`.

Both are **asynchronous**: in every language the I/O methods return whatever that language uses for
async work (`async`/`await`, `Promise`, `CompletableFuture`, `Task`, `Future`, blocking calls in Go and
Haskell) — see each guide. Both components define their own `QoS` enum (the broker's is not the
client's); where a language imports both, alias or prefix one of them.

Naming follows each language's conventions: the Rust field `keep_alive_secs` is `keep_alive_secs`
(Python, Node), `keepAliveSecs` (Kotlin, Swift, Dart, Java accessors, Go `KeepAliveSecs`),
`KeepAliveSecs` (C#). Optional values are `None` / `undefined` / `null` / `nil` / `Maybe`.

## Receiving messages and events

There are two ways, and every language supports at least one:

| Style | Client | Broker | Languages |
| --- | --- | --- | --- |
| **Callbacks** — implement an interface, the library calls you | `set_message_listener(MqttMessageListener)` | `set_event_listener(MqttBrokerEventListener)`, `set_auth_provider`, `set_enhanced_auth_provider` | all except Dart and Haskell |
| **Pull-style** — poll a queue | `enable_message_queue(capacity)` then `next_message(timeout_ms)` | `enable_event_queue(capacity)` then `next_event(timeout_ms)` | all (the only option in Dart and Haskell) |

Why Dart and Haskell can't take callbacks: Dart's VM aborts when Rust invokes a Dart callback from one
of its own threads, and the Haskell generator exposes callback interfaces only as opaque handles.
`next_message` / `next_event` return nothing (`null` / `Nothing`) when the timeout passes. Auth
providers can't be written in those two languages — use the broker's `allow_anonymous = false` rule
instead of a custom provider.

## `ConnectOptions` (client)

| Field | Meaning |
| --- | --- |
| `host`, `port` | broker address |
| `client_id` | unique per connected client |
| `version` | `V311` or `V5` |
| `clean_start` | MQTT 3.1.1 "clean session" / 5.0 "clean start" |
| `keep_alive_secs` | ping interval, `0` disables |
| `username`, `password` | optional credentials (`password` is bytes) |
| `will` | optional `WillOptions { topic, payload, qos, retain }` published by the broker if the client vanishes |
| `connect_timeout_secs` | TCP + CONNACK timeout |
| `operation_timeout_secs` | per-attempt wait for a QoS 1/2 handshake step; `0` = 15 s. A publish retries 3 times, so the worst case is 4× this |
| `auto_reconnect`, `reconnect_backoff_secs`, `reconnect_max_backoff_secs` | opt-in reconnect with exponential backoff; subscriptions are replayed (`0` = defaults 1 s / 30 s) |
| `tls` | optional `TlsOptions` (see below) |
| `max_packet_size` | largest packet accepted from the broker; `0` = protocol maximum |
| `auth_method`, `auth_data` | MQTT 5 enhanced authentication (needs `V5`) |

The last three (`max_packet_size`, `auth_method`, `auth_data`) can be left out in Python, Kotlin, Swift,
C#, Dart and Go (zero values); Java, Node and Haskell take every field explicitly (`0` / none).

## `MqttBrokerConfig` (broker)

| Field | Meaning |
| --- | --- |
| `bind_address`, `port` | TCP listener; `port = 0` picks a free port (read it with `bound_port()`) |
| `ws_port` | optional MQTT-over-WebSocket listener (`Some(0)` = free port) |
| `allow_anonymous` | accept clients without credentials when no auth provider is set |
| `max_clients` | `0` = unlimited |
| `max_qos` | highest QoS granted on SUBSCRIBE |
| `max_retained_messages`, `max_queued_per_client` | in-memory limits; `0` = unlimited |
| `redelivery_interval_secs` | resend un-acked QoS 1/2 packets; `0` = 5 s |
| `tls` | optional `BrokerTlsConfig { port, cert_pem, key_pem, client_ca_pem }` |
| `max_packet_size`, `max_outbound_queue` | hardening limits; `0` = defaults (1 MiB / 4096) |
| `session_expiry_secs` | discard offline persistent sessions after this long; `0` = never |

## Delivery results and errors

- `connect` returns `ConnectResult { session_present, reason_code }`; `subscribe` returns
  `SubscribeResult { reason_code }` — a code below `0x80` means the subscription was granted.
- A refused connection throws/returns an error: MQTT 5 reason `0x86` is "bad user name or password".
- Client errors (`MqttError`): `MalformedPacket`, `Protocol` (e.g. publishing to a wildcard topic),
  `Io`, `ConnectionRefused`, `NotConnected`, `PacketTooLarge`, `AlreadyConnected`, `Timeout`,
  `Session`. Broker errors (`MqttBrokerError`): `AlreadyRunning`, `Io`, `Tls`.
- Broker events (`BrokerEvent`, pull-style) are `ClientConnected { client_id }`,
  `ClientDisconnected { client_id, reason }`, `MessagePublished { client_id, topic, qos }`.

## Authentication

1. **Built in** — `allow_anonymous = false` refuses clients without credentials.
2. **Provider callback** — `MqttAuthProvider.authenticate(client_id, username, password) -> bool`,
   evaluated once per CONNECT; it always makes the final decision when set.
3. **Enhanced authentication (MQTT 5)** — a multi-round challenge/response (SCRAM, Kerberos, OAuth,
   …). The client sets `auth_method` / `auth_data` and an `MqttAuthHandler.respond(method, challenge)`;
   the broker registers an `MqttEnhancedAuthProvider.step(client_id, method, data, round)` that
   returns an `EnhancedAuthStep { outcome: Continue | Success | Failure, data }`. A CONNECT naming an
   authentication method is refused unless a provider is registered.

## TLS

Everything is PEM **bytes**, not file paths, so it behaves identically in every language.

- Client: `TlsOptions { ca_cert_pem, client_cert_pem, client_key_pem, insecure_skip_certificate_verification }`.
  Leaving all of them empty trusts the bundled Mozilla roots. Set `ca_cert_pem` for a private CA,
  the client pair for mutual TLS. `insecure_skip_certificate_verification` is for local development only.
- Broker: `BrokerTlsConfig { port, cert_pem, key_pem, client_ca_pem }` adds an `mqtts` listener next to
  the plain one; `client_ca_pem` turns on mutual TLS.
- TLS is pure Rust (`rustls`); there is no OpenSSL dependency.

## Shared subscriptions

Subscribe to `$share/<group>/<filter>`: the broker delivers each matching message to exactly one
member of the group, round-robin. Plain subscribers on the same filter still get every message.

## Native libraries

Every package contains or compiles the same Rust core, which is why a Rust toolchain or a prebuilt
`libmqtt_client` / `libmqtt_broker` shows up in some guides:

| Language | Native library |
| --- | --- |
| Python (wheel), Node, Kotlin, Java, C# | bundled in the package — only for the platforms the release built (Linux x86_64 today; Android AAR has four ABIs) |
| Swift | XCFrameworks inside the zip (macOS, iOS, simulator) |
| Dart, Haskell | built from the bundled Rust sources at install time (needs `cargo`) |
| Go | installed separately (release assets or `cargo build --release`) and found with `CGO_LDFLAGS` |
| Rust | it *is* the library |

## A complete example per language

Each language has an end-to-end test that runs on every push, against real sockets: a broker with an
auth provider and an event listener, a QoS 1 publish received through a callback, a bad login
refused, and the broker events observed. They are the most reliable reference for the exact API in
that language: [`tests/bindings/`](../../tests/bindings).
