// Runtime smoke test for the Dart bindings: client + broker over real sockets.
// Prints "SMOKE OK" on success.
//
// uniffi-dart cannot service callbacks that Rust invokes from its own threads
// (the VM aborts with "Cannot invoke native callback outside an isolate"), so
// MqttMessageListener, MqttAuthProvider and MqttBrokerEventListener are
// unusable from Dart. This test therefore uses the *pull-style* API instead:
// `enableMessageQueue`/`nextMessage` and `enableEventQueue`/`nextEvent`.
// Authentication goes through the built-in `allowAnonymous: false` rule.
import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import 'package:stem_mqtt/mqtt_broker.dart' as broker_api;
import 'package:stem_mqtt/mqtt_client.dart';
// Each component owns its QoS, so the broker's is reached through the prefix.

void must(bool cond, String what) {
  if (!cond) {
    stderr.writeln('FAIL $what');
    exit(1);
  }
}

ConnectOptions options(int port, String id, [String? user]) => ConnectOptions(
      host: '127.0.0.1',
      port: port,
      clientId: id,
      version: MqttVersion.v5,
      cleanStart: true,
      keepAliveSecs: 30,
      username: user,
      connectTimeoutSecs: 10,
      operationTimeoutSecs: 5,
      autoReconnect: false,
      reconnectBackoffSecs: 0,
      reconnectMaxBackoffSecs: 0,
    );

Future<void> main() async {
  final broker = broker_api.MqttBroker(
      config: broker_api.MqttBrokerConfig(
    bindAddress: '127.0.0.1',
    port: 0,
    allowAnonymous: false,
    maxClients: 0,
    maxQos: broker_api.QoS.exactlyOnce,
    maxRetainedMessages: 100,
    maxQueuedPerClient: 100,
    redeliveryIntervalSecs: 0,
  ));
  broker.enableEventQueue(capacity: 16);
  await broker.start();
  must(broker.isRunning(), 'broker running');
  final port = broker.boundPort();
  must(port != null && port != 0, 'bound port');

  final sub = MqttClient(options: options(port!, 'dart-sub', 'user'));
  sub.enableMessageQueue(capacity: 16); // pull, don't push
  final connected = await sub.connect();
  must(connected.reasonCode == 0, 'connect accepted');
  must(sub.isConnected(), 'client reports connected');
  final granted = await sub.subscribe(topicFilter: 'smoke/#', qos: QoS.atLeastOnce);
  must(granted.reasonCode < 0x80, 'subscribe granted');

  final pub = MqttClient(options: options(port, 'dart-pub', 'user'));
  await pub.connect();
  must(broker.clientCount() == 2, 'broker counts both clients');
  // QoS 1 completes only after the broker's PUBACK, so this proves the whole
  // encode -> socket -> broker -> ack -> decode path through the FFI.
  await pub.publish(
      topic: 'smoke/dart',
      payload: Uint8List.fromList(utf8.encode('hello-dart')),
      qos: QoS.atLeastOnce,
      retain: false);

  // The message arrives through the queue — no callback involved.
  final message = await sub.nextMessage(timeoutMs: 5000);
  must(message != null, 'message delivered');
  must(message!.topic == 'smoke/dart' && utf8.decode(message.payload) == 'hello-dart',
      'message content');

  var refused = false;
  try {
    await MqttClient(options: options(port, 'dart-bad')).connect();
  } catch (_) {
    refused = true;
  }
  must(refused, 'broker must refuse an anonymous client when allowAnonymous is false');

  // The broker's events are pulled the same way.
  final seen = <String>{};
  for (broker_api.BrokerEvent? e = await broker.nextEvent(timeoutMs: 300);
      e != null;
      e = await broker.nextEvent(timeoutMs: 300)) {
    if (e is broker_api.ClientConnectedBrokerEvent) seen.add(e.clientId);
  }
  must(seen.containsAll(['dart-sub', 'dart-pub']), 'broker event queue saw the connections');

  await pub.disconnect();
  await sub.disconnect();
  await broker.stop();
  must(!broker.isRunning(), 'broker stopped');
  print('SMOKE OK');
  exit(0);
}
