# stem_mqtt

MQTT 3.1.1 / 5.0 client **and broker** for Dart, with a Rust core, via generated
[UniFFI](https://mozilla.github.io/uniffi-rs/) bindings.

```dart
import 'package:stem_mqtt/mqtt_broker.dart' as broker_api;
import 'package:stem_mqtt/mqtt_client.dart';
```

* **Requires a Rust toolchain** (`cargo`): the package builds its bundled Rust sources the first
  time it is used (a native-assets build hook), for the machine running Dart.
* **Callbacks:** Rust cannot call back into Dart from its own threads, so use the pull-style API
  (`enableMessageQueue` / `nextMessage`, `enableEventQueue` / `nextEvent`) instead of listeners.
* The client and broker define their own `QoS` enums — import them with a prefix.

Source and issues: <https://github.com/sorinirimies/stem-mqtt>
