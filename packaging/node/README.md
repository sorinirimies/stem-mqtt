# Node.js / TypeScript packaging (npm, via napi-rs)

UniFFI (used here for Kotlin, Swift, and Python) doesn't target JavaScript —
its C-ABI FFI model maps naturally onto the JVM/Swift/CPython/MRI runtimes,
but not onto V8/Node. Node.js bindings instead come from
[`crates/mqtt-client-node`](../../crates/mqtt-client-node), a separate crate
built with [napi-rs](https://napi.rs/): a native Node addon (`.node` file)
plus a hand-written wrapper (`src/lib.rs`) around `mqtt_client::MqttClient`.

## Scope — what this does and doesn't cover

- **Covers**: Node.js (server-side, CLI tools, Electron main process) — the
  addon wraps the existing `tokio`-based client, including TCP, TLS/mTLS,
  automatic reconnect, and subscription replay.
- **Does not cover the browser.** Browsers can't open raw TCP sockets. A
  browser build would need a WebSocket transport in `mqtt-client`'s connection
  layer; this addon does not fake one.
- Only `mqtt-client` is wrapped (a Node embedded broker is a much rarer use
  case than a Node client) — same reasoning as the Swift package.

## Callback convention

`setMessageListener` callbacks are Node-style **error-first**:
`(err, value) => void`. This falls out of napi-rs's "callee-handled"
`ThreadsafeFunction` mode; `err` is always `null` here (delivery/disconnect
notifications can't themselves fail) — see `index.d.ts` for the exact
generated types.

```ts
import { MqttClient } from "stem-mqtt-client";

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
addon for Linux x86_64/aarch64, macOS x86_64/arm64, and Windows x86_64 (one
npm package bundling all five `.node` files — the generated `index.js` picks
the right one at `require()` time; no per-platform `optionalDependencies`
split yet, that's a reasonable follow-up for smaller install sizes).
Linux aarch64 builds on an `ubuntu-latest` (x64) runner via napi-rs's own
`--use-napi-cross` flag, which downloads a prebuilt glibc cross-toolchain
from `@napi-rs/cross-toolchain` — no Docker/`cross`-rs container or local
`zig` install needed (N-API's headers are vendored in `napi-sys`, so this
is just a normal cdylib cross-compile under the hood, same idea as
`build-broker`'s `cross`-based aarch64 job, just via napi-rs's own
toolchain fetcher instead). `publish-node` then `npm publish`es, gated on
the `NPM_TOKEN` repository secret:

```sh
gh secret set NPM_TOKEN   # use a granular token allowed to publish this package with 2FA bypass
```

npm rejects ordinary tokens when the account/package requires 2FA. The token
must have package write access and `bypass 2FA` enabled; the previous token
failed with npm `E403` for this reason.

If unset, the job skips with a message instead of failing, same pattern as
crates.io/PyPI.

## Caveats

- This is a hand-written wrapper (not auto-generated the way UniFFI's
  Kotlin/Swift/Python bindings are) — if `mqtt_client::client`'s public API
  changes, `crates/mqtt-client-node/src/lib.rs` needs a matching manual
  update.
