# stem_mqtt

An MQTT 3.1.1 / 5.0 **client and embeddable broker** for Elixir and Erlang, implemented in Rust
([stem-mqtt](https://github.com/sorinirimies/stem-mqtt)) and shipped as a precompiled NIF: no Rust
toolchain needed.

```elixir
{:stem_mqtt, "~> 0.5"}
```

```elixir
{:ok, broker} = StemMqtt.Broker.new(port: 0)
:ok = StemMqtt.Broker.start(broker)

{:ok, client} = StemMqtt.Client.new(host: "127.0.0.1", port: StemMqtt.Broker.port(broker))
{:ok, _} = StemMqtt.Client.connect(client)
{:ok, _} = StemMqtt.Client.subscribe(client, "sensors/#", qos: 1)
:ok = StemMqtt.Client.publish(client, "sensors/t", "21.5", qos: 1)

id = client.id
receive do
  {:stem_mqtt, ^id, {:message, %StemMqtt.Message{topic: t, payload: p}}} -> {t, p}
end
```

- The creating process owns a client or broker: messages, disconnects, broker events and auth
  requests arrive in its mailbox, and when it exits the client disconnects / the broker stops.
- Calls never block a BEAM scheduler; async work runs on a Tokio runtime inside the NIF.
- TLS (rustls), MQTT over WebSocket (broker), QoS 0/1/2, retained messages, wills, persistent sessions.
- An Erlang API too: the `stem_mqtt` module.

Precompiled for Linux x86_64/aarch64 (glibc), macOS x86_64/arm64 and Windows x86_64; OTP 25+.

Guide: [docs/languages/elixir.md](https://github.com/sorinirimies/stem-mqtt/blob/main/docs/languages/elixir.md).
License: MIT.
