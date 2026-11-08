# Ruby

UniFFI's built-in Ruby bindings, published as two gems. Read the [shared concepts](README.md) first;
this page covers what is Ruby-specific.

## Install

```sh
gem install stem-mqtt-client   # the client
gem install stem-mqtt-broker   # the embeddable broker
```

Each gem is built per platform and contains only that platform's native library: Linux x86_64 /
aarch64 (glibc), macOS x86_64 / arm64, Windows x64 (RubyInstaller UCRT). Requires Ruby 3.1+ and the
`ffi` gem (installed as a dependency).

## Use

```ruby
require "stem_mqtt_client"

class Inbox
  def on_message(message) = puts "#{message.topic}: #{message.payload}"
  def on_disconnected(reason) = warn "disconnected: #{reason}"
end

options = MqttClient::ConnectOptions.new(
  host: "localhost", port: 1883, client_id: "ruby-1", version: MqttClient::MqttVersion::V5,
  clean_start: true, keep_alive_secs: 30, username: nil, password: nil, will: nil,
  connect_timeout_secs: 10, operation_timeout_secs: 5, auto_reconnect: false,
  reconnect_backoff_secs: 0, reconnect_max_backoff_secs: 0, tls: nil
)
client = MqttClient::MqttClient.new(options)
client.set_message_listener(Inbox.new)
client.connect
client.subscribe("sensors/#", MqttClient::QoS::AT_LEAST_ONCE)
client.publish("sensors/t", "21.5", MqttClient::QoS::AT_LEAST_ONCE, false)
```

- **Async methods block** the calling thread until Rust finishes, releasing the GVL meanwhile; under a
  `Fiber::Scheduler` (e.g. the `async` gem) they yield the fiber instead.
- **Callback interfaces are duck-typed:** pass any object with the listed methods.
- **Records are immutable** and take keyword arguments; fields with defaults may be omitted.
- **Errors** are raised as exceptions under `MqttClient::MqttError` / `MqttBroker::MqttBrokerError`.
- The broker gem is self-contained: `require "stem_mqtt_broker"`, then `MqttBroker::MqttBroker`.

The scenario in [`tests/bindings/ruby/smoke.rb`](../../tests/bindings/ruby/smoke.rb) exercises the
broker, callbacks in both directions, pull-style delivery and MQTT 5 enhanced auth.
