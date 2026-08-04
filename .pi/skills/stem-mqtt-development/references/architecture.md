# Architecture reference — stem-mqtt

## Workspace layout

- `crates/mqtt-client` hosts the async `tokio` client **and** the shared
  wire-protocol codec (`mqtt_client::protocol`) used by both the client and
  the broker — there is intentionally no separate "core" crate.
  `crates/mqtt-broker` reuses that codec instead of duplicating it.
- `crates/mqtt-client-node` is a hand-written napi-rs addon (not a UniFFI
  target, since JS isn't one).

## Client internals (`crates/mqtt-client/src/client/`)

Split by responsibility across `{mod,types,inner,io}.rs`:

- `types.rs` — pure data (`ConnectOptions`, `MqttMessage`, listener trait),
  no logic.
- `inner.rs` — live-connection state (`Inner`) + the pending-ack/QoS-2-dedup
  bookkeeping built directly on it.
- `io.rs` — wire I/O: the background read loop, keep-alive loop, and packet
  encode/write helpers.
- `mod.rs` — the public `MqttClient` API only
  (`new`/`connect`/`publish`/`subscribe`/`unsubscribe`/`disconnect`).

Follow this split when adding client functionality — don't dump everything
back into one file.

## Broker internals (`crates/mqtt-broker/src/`)

Split into cohesive subsystems instead of one flat `BrokerState`:

- `retain.rs` — `RetainStore`: the retained-message store, and only that.
- `registry.rs` — `SessionRegistry`: owns the `client_id -> Session` map,
  per-client send/queue/QoS-2 bookkeeping, and fan-out (matching a
  published message to subscribers).
- `events.rs` — `EventHub`: connect/disconnect/publish notifications to the
  foreign-side listener.
- `broker.rs` — `BrokerState` composes the three subsystems above, plus
  *only* the operations that genuinely span more than one of them
  (`detach_session`'s will handling: registry + retain + fan-out;
  `send_matching_retained`: retain lookup + registry delivery).
- `connection.rs` — per-connection task (CONNECT handshake, read loop,
  packet dispatch). Calls `state.sessions.x()` / `state.retained.x()` /
  `state.events.x()` directly for single-subsystem operations — only reach
  for a `BrokerState` method when an operation genuinely spans more than
  one subsystem.
- `session.rs` — `Session` (per-client state: subscriptions, offline
  queue, QoS-2 dedup maps), `Subscription`, `QueuedMessage`.
- `topic.rs` — topic name/filter matching (`+`/`#` wildcards), independent
  of everything else.
- `ws.rs` — MQTT-over-WebSocket transport adapter, so the same
  `connection.rs` logic drives both raw TCP and WebSocket connections.

When adding a broker feature, ask "which subsystem does this belong to?"
before writing code — resist the urge to grow `BrokerState` back into a
god object.
