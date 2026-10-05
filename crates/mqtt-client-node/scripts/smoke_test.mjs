#!/usr/bin/env node
// End-to-end smoke test for the mqtt-client-node addon: connects a real
// MqttClient to a real mqtt-broker (must already be listening — see
// `npm run build:debug && cargo run -p stem-mqtt-broker -- --port 18830` or the
// `test-node` CI job), subscribes, publishes, and asserts the message
// comes back through the (error-first) message listener callback.
//
// Usage: node scripts/smoke_test.mjs [port]

import { MqttClient } from "../index.js";

const port = Number(process.argv[2] ?? 18830);

async function assertRejects(what, fn) {
  try {
    await fn();
  } catch {
    return;
  }
  throw new Error(`${what}: expected a rejection`);
}

function assertThrows(what, fn) {
  try {
    fn();
  } catch {
    return;
  }
  throw new Error(`${what}: expected an exception`);
}
const topic = "stem-mqtt/node-smoke-test";
const payload = "hello from node";

const received = [];
const client = new MqttClient({
  host: "127.0.0.1",
  port,
  clientId: "node-smoke-test",
  version: "5.0",
});

client.setMessageListener(
  (_err, message) => received.push(message),
  (_err, _reason) => {},
);

const connectResult = await client.connect();
if (connectResult.reasonCode !== 0) {
  throw new Error(`connect failed: reasonCode=${connectResult.reasonCode}`);
}

const subscribeResult = await client.subscribe(topic, 1);
if (subscribeResult.reasonCode >= 0x80) {
  throw new Error(`subscribe rejected: reasonCode=${subscribeResult.reasonCode}`);
}

await client.publish(topic, Buffer.from(payload), 1, false);

// Input validation surfaces as a rejected promise / thrown error, not a dropped connection.
await assertRejects("wildcard publish topic", () =>
  client.publish("bad/+/topic", Buffer.from("x"), 0, false),
);
await assertRejects("QoS 3", () => client.publish(topic, Buffer.from("x"), 3, false));
assertThrows("unknown protocol version", () =>
  new MqttClient({ host: "127.0.0.1", port, clientId: "v", version: "4" }),
);
if (!client.isConnected()) throw new Error("local validation must not cost us the connection");

// Give the read loop a moment to deliver the message before checking.
await new Promise((resolve) => setTimeout(resolve, 500));

await client.disconnect();

if (received.length !== 1) {
  throw new Error(`expected 1 message, got ${received.length}`);
}
if (received[0].topic !== topic || received[0].payload.toString() !== payload) {
  throw new Error(`unexpected message: ${JSON.stringify(received[0])}`);
}

console.log("PASS: mqtt-client-node round-trip OK");
