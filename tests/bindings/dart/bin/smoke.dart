// Runtime smoke test for the Dart bindings: client + broker over real sockets.
// Prints "SMOKE OK" on success.
//
// KNOWN UPSTREAM LIMITATION (uniffi-dart): foreign callback interfaces cannot
// be invoked from Rust's own threads — the Dart VM aborts with "Cannot invoke
// native callback outside an isolate". That rules out every callback in this
// API: MqttMessageListener, MqttAuthProvider and MqttBrokerEventListener. So
// unlike the other languages' smoke tests this one deliberately uses none of
// them, and verifies delivery indirectly: broker-side client counts, SUBACK,
// and QoS 1 PUBACK round trips. Authentication goes through the built-in
// `allowAnonymous: false` rule instead of a provider.
import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import 'package:uniffi/mqtt_broker.dart';
import 'package:uniffi/mqtt_client.dart';

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
  final broker = MqttBroker(
      config: MqttBrokerConfig(
    bindAddress: '127.0.0.1',
    port: 0,
    allowAnonymous: false,
    maxClients: 0,
    maxQos: QoS.exactlyOnce,
    maxRetainedMessages: 100,
    maxQueuedPerClient: 100,
    redeliveryIntervalSecs: 0,
  ));
  await broker.start();
  must(broker.isRunning(), 'broker running');
  final port = broker.boundPort();
  must(port != null && port != 0, 'bound port');

  final sub = MqttClient(options: options(port!, 'dart-sub', 'user'));
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

  var refused = false;
  try {
    await MqttClient(options: options(port, 'dart-bad')).connect();
  } catch (_) {
    refused = true;
  }
  must(refused, 'broker must refuse an anonymous client when allowAnonymous is false');

  await pub.disconnect();
  await sub.disconnect();
  await broker.stop();
  must(!broker.isRunning(), 'broker stopped');
  print('SMOKE OK');
  exit(0);
}
