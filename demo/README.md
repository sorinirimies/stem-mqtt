# stem-mqtt demo

A real, browser-based MQTT client for exercising a running `mqtt-broker`:
connect, publish, subscribe, and compare QoS 0/1/2 delivery — built with
[Topcoat](https://github.com/topcoat/topcoat) (CSS) and
[mqtt.js](https://github.com/mqttjs/MQTT.js) (MQTT-over-WebSocket in the
browser), served as plain static files (`index.html`/`app.js`/`style.css`,
no build step).

This only works because the broker speaks **MQTT-over-WebSocket**
(MQTT-5.0 §6 / MQTT-3.1.1 Appendix B) in addition to raw TCP — browsers
can't open raw TCP sockets, so `mqtt-broker --ws-port <port>` bridges
WebSocket connections into the exact same connection handling as TCP ones
(see `crates/mqtt-broker/src/ws.rs`).

## Quickest path: plain binaries

```sh
cargo run -p mqtt-broker --bin mqtt-broker -- --ws-port 8083
cd demo/web && python3 -m http.server 8090   # or any static file server
open http://localhost:8090
```

The page's default WebSocket URL (`ws://localhost:8083`) already matches.

## Docker Compose

```sh
docker compose up --build
open http://localhost:8090
```

Builds and runs both the broker (`packaging/Dockerfile`, ports `1883` +
`8083`) and an nginx container serving `demo/web/` (`demo/web/Dockerfile`,
port `8090`) — see [`docker-compose.yml`](../docker-compose.yml).

## Kubernetes

See [`packaging/k8s/README.md`](../packaging/k8s/README.md) — a one-Pod
quick demo, or separate Deployments/Services for broker and demo-web.

## What to try

- Connect, then **Subscribe** to `stem-mqtt/#` (or any filter).
- **Publish** to `stem-mqtt/demo` and watch it arrive in the activity log.
- **Publish at all 3 QoS levels** publishes the same payload at QoS 0, 1,
  and 2 back-to-back and times each ack — QoS 0 is a fire-and-forget
  (no ack wait), QoS 1 waits for PUBACK, QoS 2 waits for the full
  PUBREC/PUBREL/PUBCOMP handshake, so the timings should visibly increase.
- Toggle **Retain**, publish, then open a second tab and subscribe fresh —
  the retained message arrives immediately to the new subscriber.
- Everything is logged with timestamps: connects, subscribes, every
  publish's ack latency, every inbound message.
