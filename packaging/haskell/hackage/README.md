# stem-mqtt (Haskell)

MQTT 3.1.1 / 5.0 client **and broker**, with a Rust core, via generated
[UniFFI](https://mozilla.github.io/uniffi-rs/) bindings.

```haskell
import qualified UniFFI.MqttBroker as Broker
import qualified UniFFI.MqttClient as Client
```

A complete broker + client round trip (publish, subscribe, pull-style receive, enhanced auth) is in
[`tests/bindings/haskell/Main.hs`](https://github.com/sorinirimies/stem-mqtt/blob/main/tests/bindings/haskell/Main.hs).

* **Requires a Rust toolchain** (`cargo`): the package builds its bundled Rust sources at install time.
* **Callbacks:** generated Haskell bindings can't implement Rust callback interfaces, so use
  `mqttClientEnableMessageQueue`/`mqttClientNextMessage` and `mqttBrokerEnableEventQueue`/`mqttBrokerNextEvent`.
* Bundles `UniFFI.Runtime` from [uniffi-bindgen-haskell](https://github.com/mercury/uniffi-bindgen-haskell)
  (Apache-2.0, see `LICENSE.uniffi-runtime`).

Source and issues: <https://github.com/sorinirimies/stem-mqtt>
