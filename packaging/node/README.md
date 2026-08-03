# Node.js / TypeScript packaging (npm, via napi-rs)

UniFFI (Kotlin/Swift/Python/Ruby's binding path) doesn't target JavaScript —
its C-ABI FFI model maps naturally onto the JVM/Swift/CPython/MRI runtimes,
but not onto V8/Node. Node.js bindings instead come from
[`crates/mqtt-client-node`](../../crates/mqtt-client-node), a separate crate
built with [napi-rs](https://napi.rs/): a native Node addon (`.node` file)
plus a hand-written wrapper (`src/lib.rs`) around `mqtt_client::MqttClient`.

## Scope — what this does and doesn't cover

- **Covers**: Node.js (server-side, CLI tools, Electron main process) — the
  addon wraps the existing `tokio`-based client as-is, including real TCP
  sockets.
- **Does not cover the browser.** Browsers can't open raw TCP sockets, and
  this project's client/broker only speak plain MQTT-over-TCP today (no
  MQTT-over-WebSocket framing). A browser build would need a WebSocket
  transport added to `mqtt-client`'s connection layer first — worth doing
  as a follow-up, not something this crate fakes.
- Only `mqtt-client` is wrapped (a Node embedded broker is a much rarer use
  case than a Node client) — same reasoning as the Swift package.

## Callback convention

`setMessageListener` callbacks are Node-style **error-first**:
`(err, value) => void`. This falls out of napi-rs's "callee-handled"
`ThreadsafeFunction` mode; `err` is always `null` here (delivery/disconnect
notifications can't themselves fail) — see `index.d.ts` for the exact
generated types.

```ts
import { MqttClient } from "@stem-mqtt/client";

const client = new MqttClient({ host: "localhost", port: 1883, clientId: "demo", version: "5.0" });
client.setMessageListener(
  (_err, message) => console.log(message.topic, message.payload.toString()),
  (_err, reason) => console.log("disconnected:", reason),
);
await client.connect();
await client.subscribe("demo/topic", 1);
await client.publish("demo/topic", Buffer.from("hi"), 1, false);
```

## Building locally

```sh
cd crates/mqtt-client-node
npm ci
npm run build:debug      # fast iteration: dev profile, current platform only
npm run build            # release profile, current platform
node scripts/smoke_test.mjs   # needs a broker already listening on :18830
```

## Publishing (CI)

`.github/workflows/release.yml`'s `build-node` job cross-builds the native
addon for Linux x86_64, macOS x86_64/arm64, and Windows x86_64 (one npm
package bundling all four `.node` files — the generated `index.js` picks
the right one at `require()` time; no per-platform `optionalDependencies`
split yet, that's a reasonable follow-up for smaller install sizes).
`publish-node` then `npm publish`es, gated on the `NPM_TOKEN` repository
secret:

```sh
gh secret set NPM_TOKEN   # paste an npm "Automation" token when prompted
```

If unset, the job skips with a message instead of failing, same pattern as
crates.io/PyPI/RubyGems.

## Caveats

- Linux aarch64 isn't cross-compiled for Node yet (napi-rs cross-compiling
  a native addon typically wants `zig` or a `cross`-style container with
  Node headers baked in — deferred to keep this addition's scope bounded;
  `build-broker`'s existing `cross` setup doesn't directly carry over since
  it targets a plain binary, not a Node native module).
- This is a hand-written wrapper (not auto-generated the way UniFFI's
  Kotlin/Swift/Python bindings are) — if `mqtt_client::client`'s public API
  changes, `crates/mqtt-client-node/src/lib.rs` needs a matching manual
  update.
