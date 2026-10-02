// Runtime smoke test for the Node.js bindings (uniffi-bindgen-node-js): client
// only — that generator cannot emit the broker — so it talks to the real
// `mqtt-broker` binary (path in STEM_MQTT_BROKER_BIN), which this script starts
// and stops itself.
// Prints "SMOKE OK" on success.
// Scenario: subscribe, publish QoS 1, receive via the listener callback,
// check error mapping, disconnect.
import { spawn } from "node:child_process";
import { MqttClient, MqttVersion, QoS, MqttErrorProtocol } from "./index.js";

const bin = process.env.STEM_MQTT_BROKER_BIN;
if (!bin) throw new Error("STEM_MQTT_BROKER_BIN is not set");
const port = 20000 + Math.floor(Math.random() * 20000);
const broker = spawn(bin, ["--port", String(port), "--log-level", "warn"], { stdio: "inherit" });
const done = (code) => { broker.kill(); process.exit(code); };
await new Promise((r) => setTimeout(r, 1000)); // let it bind

function must(cond, what) {
  if (!cond) {
    console.error(`FAIL ${what}`);
    done(1);
  }
}

const options = (client_id) => ({
  host: "127.0.0.1", port, client_id, version: MqttVersion.V5, clean_start: true,
  keep_alive_secs: 30, username: undefined, password: undefined, will: undefined,
  connect_timeout_secs: 10, operation_timeout_secs: 5, auto_reconnect: false,
  reconnect_backoff_secs: 0, reconnect_max_backoff_secs: 0, tls: undefined,
  max_packet_size: 0,
});

const sub = new MqttClient(options("node-sub"));
const received = new Promise((resolve) => {
  sub.set_message_listener({
    on_message: (message) => resolve(message),
    on_disconnected: () => {},
  });
});
await sub.connect();
must(sub.is_connected(), "client reports connected");
const granted = await sub.subscribe("smoke/#", QoS.AtLeastOnce);
must(granted.reason_code < 0x80, "subscribe granted");

const pub = new MqttClient(options("node-pub"));
await pub.connect();
await pub.publish("smoke/node", new TextEncoder().encode("hello-node"), QoS.AtLeastOnce, false);

const message = await Promise.race([
  received,
  new Promise((_, reject) => setTimeout(() => reject(new Error("timeout")), 5000)),
]);
must(message.topic === "smoke/node", "message topic");
must(new TextDecoder().decode(message.payload) === "hello-node", "message payload");

let mapped = false;
try {
  await pub.publish("bad/+/topic", new Uint8Array(), QoS.AtMostOnce, false);
} catch (e) {
  mapped = e instanceof MqttErrorProtocol;
}
must(mapped, "wildcard publish must surface as MqttErrorProtocol");

await pub.disconnect();
await sub.disconnect();
console.log("SMOKE OK");
done(0);
