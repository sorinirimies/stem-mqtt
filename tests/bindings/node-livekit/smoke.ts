// Runtime smoke test for the livekit `uniffi-bindgen-node` bindings (client
// only, no callbacks — that generator doesn't support them). Starts the real
// `mqtt-broker` binary (STEM_MQTT_BROKER_BIN) itself. Prints "SMOKE OK".
import { spawn } from 'node:child_process';
import { MqttClient } from './index';

const bin = process.env.STEM_MQTT_BROKER_BIN;
if (!bin) throw new Error('STEM_MQTT_BROKER_BIN is not set');
const port = 20000 + Math.floor(Math.random() * 20000);
const broker = spawn(bin, ['--port', String(port), '--log-level', 'warn'], { stdio: 'inherit' });
const done = (code: number) => { broker.kill(); process.exit(code); };

const opts = (id: string) => ({
  host: '127.0.0.1', port, clientId: id, version: 'v5' as const, cleanStart: true, keepAliveSecs: 30,
  username: undefined, password: undefined, will: undefined, connectTimeoutSecs: 10,
  operationTimeoutSecs: 5, autoReconnect: false, reconnectBackoffSecs: 0, reconnectMaxBackoffSecs: 0,
  tls: undefined, maxPacketSize: 0,
});

(async () => {
  await new Promise((r) => setTimeout(r, 1000));
  const sub = new MqttClient(opts('lk-sub'));
  await sub.connect();
  const granted = await sub.subscribe('smoke/#', 'atLeastOnce');
  if (granted.reasonCode >= 0x80) throw new Error('subscribe refused');
  const pub = new MqttClient(opts('lk-pub'));
  await pub.connect();
  await pub.publish('smoke/lk', new Uint8Array([104, 105]).buffer, 'atLeastOnce', false);
  await pub.disconnect();
  await sub.disconnect();
  console.log('SMOKE OK');
  done(0);
})().catch((e) => { console.error('FAIL', e); done(1); });
