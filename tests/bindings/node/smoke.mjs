// Runtime smoke test for the Node.js bindings (uniffi-bindgen-node-js): client +
// broker, both callback directions, real sockets. Prints "SMOKE OK" on success.
// Scenario is identical in every language's smoke test (see tests/bindings/python/smoke.py).
//
// The runner stages the two generated packages as ./client and ./broker.
import * as mqtt from "./client/index.js";
import * as brk from "./broker/index.js";

function must(cond, what) {
  if (!cond) {
    console.error(`FAIL ${what}`);
    process.exit(1);
  }
}

const options = (client_id, extra = {}) => ({
  host: "127.0.0.1", port: 0, client_id, version: mqtt.MqttVersion.V5, clean_start: true,
  keep_alive_secs: 30, username: undefined, password: undefined, will: undefined,
  connect_timeout_secs: 10, operation_timeout_secs: 5, auto_reconnect: false,
  reconnect_backoff_secs: 0, reconnect_max_backoff_secs: 0, tls: undefined,
  max_packet_size: 0, auth_method: undefined, auth_data: undefined, ...extra,
});

const connected = new Set();
const broker = new brk.MqttBroker({
  bind_address: "127.0.0.1", port: 0, ws_port: undefined, allow_anonymous: true, max_clients: 0,
  max_qos: brk.QoS.ExactlyOnce, max_retained_messages: 100, max_queued_per_client: 100,
  redelivery_interval_secs: 0, tls: undefined, max_packet_size: 0, max_outbound_queue: 0,
  session_expiry_secs: 0,
});
broker.set_auth_provider({ authenticate: (_id, username) => username !== "bad" });
broker.set_event_listener({
  on_client_connected: (id) => connected.add(id),
  on_client_disconnected: () => {},
  on_message_published: () => {},
});
await broker.start();
must(broker.is_running(), "broker running");
const port = broker.bound_port();
must(port, "bound port");

const withPort = (id, extra) => options(id, { port, ...extra });

let resolveMessage;
const received = new Promise((r) => (resolveMessage = r));
const sub = new mqtt.MqttClient(withPort("node-sub"));
sub.set_message_listener({ on_message: (m) => resolveMessage(m), on_disconnected: () => {} });
await sub.connect();
must(sub.is_connected(), "client reports connected");
const granted = await sub.subscribe("smoke/#", mqtt.QoS.AtLeastOnce);
must(granted.reason_code < 0x80, "subscribe granted");

const pub = new mqtt.MqttClient(withPort("node-pub"));
await pub.connect();
await pub.publish("smoke/node", new TextEncoder().encode("hello-node"), mqtt.QoS.AtLeastOnce, false);

const message = await Promise.race([
  received,
  new Promise((_, reject) => setTimeout(() => reject(new Error("timeout")), 5000)),
]);
must(message.topic === "smoke/node", "message topic");
must(new TextDecoder().decode(message.payload) === "hello-node", "message payload");

let refused = false;
try {
  await new mqtt.MqttClient(withPort("node-bad", { username: "bad" })).connect();
} catch {
  refused = true;
}
must(refused, "auth provider must refuse bad credentials");
must(connected.has("node-sub") && connected.has("node-pub"), "event listener saw connections");

let mapped = false;
try {
  await pub.publish("bad/+/topic", new Uint8Array(), mqtt.QoS.AtMostOnce, false);
} catch (e) {
  mapped = e instanceof mqtt.MqttErrorProtocol;
}
must(mapped, "wildcard publish must surface as MqttErrorProtocol");

// Pull-style delivery (the path Dart/Haskell use) works from Node too.
const polled = new mqtt.MqttClient(withPort("node-polled"));
polled.enable_message_queue(8);
await polled.connect();
await polled.subscribe("smoke/polled", mqtt.QoS.AtLeastOnce);
await pub.publish("smoke/polled", new TextEncoder().encode("pulled"), mqtt.QoS.AtLeastOnce, false);
const pulled = await polled.next_message(5000);
must(pulled && new TextDecoder().decode(pulled.payload) === "pulled", "polled message");
await polled.disconnect();

// MQTT 5 enhanced authentication: challenge/response through foreign callbacks
// in both directions (broker provider + client handler).
broker.set_enhanced_auth_provider({
  step: (_id, _method, data, round) =>
    round === 0
      ? { outcome: brk.EnhancedAuthOutcome.Continue, data: new Uint8Array([7]) }
      : data && data[0] === 8
        ? { outcome: brk.EnhancedAuthOutcome.Success, data: new TextEncoder().encode("welcome") }
        : { outcome: brk.EnhancedAuthOutcome.Failure, data: new Uint8Array() },
});
const eauth = new mqtt.MqttClient(
  withPort("node-eauth", { auth_method: "X-ADD-ONE", auth_data: new TextEncoder().encode("hello") }),
);
eauth.set_auth_handler({ respond: (_method, challenge) => challenge.map((b) => b + 1) });
await eauth.connect();
must(eauth.is_connected(), "enhanced auth connected");
await eauth.disconnect();

await pub.disconnect();
await sub.disconnect();
await broker.stop();
must(!broker.is_running(), "broker stopped");
console.log("SMOKE OK");
process.exit(0);
