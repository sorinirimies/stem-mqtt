// Runtime smoke test for the Swift bindings: client + broker, both callback
// directions, real sockets. Prints "SMOKE OK" on success.
// Scenario is identical in every language's smoke test (see tests/bindings/python/smoke.py).
import Foundation
import MqttBroker
import MqttClient

func must(_ cond: Bool, _ what: String) {
    if !cond {
        FileHandle.standardError.write(Data("FAIL \(what)\n".utf8))
        exit(1)
    }
}

final class Inbox: MqttMessageListener, @unchecked Sendable {
    private let lock = NSLock()
    private var messages: [MqttMessage] = []
    func onMessage(message: MqttMessage) { lock.lock(); messages.append(message); lock.unlock() }
    func onDisconnected(reason: String) {}
    func first() -> MqttMessage? { lock.lock(); defer { lock.unlock() }; return messages.first }
}

final class Auth: MqttAuthProvider, @unchecked Sendable {
    func authenticate(clientId: String, username: String?, password: Data?) -> Bool { username != "bad" }
}

final class Events: MqttBrokerEventListener, @unchecked Sendable {
    private let lock = NSLock()
    private var ids: Set<String> = []
    func onClientConnected(clientId: String) { lock.lock(); ids.insert(clientId); lock.unlock() }
    func onClientDisconnected(clientId: String, reason: String) {}
    func onMessagePublished(clientId: String, topic: String, qos: QoS) {}
    func connected() -> Set<String> { lock.lock(); defer { lock.unlock() }; return ids }
}

func options(_ port: UInt16, _ id: String, user: String? = nil) -> ConnectOptions {
    ConnectOptions(
        host: "127.0.0.1", port: port, clientId: id, version: .v5, cleanStart: true,
        keepAliveSecs: 30, username: user, password: nil, will: nil, connectTimeoutSecs: 10,
        operationTimeoutSecs: 5, autoReconnect: false, reconnectBackoffSecs: 0,
        reconnectMaxBackoffSecs: 0, tls: nil)
}

let events = Events()
let broker = MqttBroker(config: MqttBrokerConfig(
    bindAddress: "127.0.0.1", port: 0, wsPort: nil, allowAnonymous: true, maxClients: 0,
    maxQos: .exactlyOnce, maxRetainedMessages: 100, maxQueuedPerClient: 100,
    redeliveryIntervalSecs: 0, tls: nil))
broker.setAuthProvider(provider: Auth())
broker.setEventListener(listener: events)
try await broker.start()
must(broker.isRunning(), "broker running")
let port = broker.boundPort() ?? 0
must(port != 0, "bound port")

let inbox = Inbox()
let sub = MqttClient(options: options(port, "swift-sub"))
sub.setMessageListener(listener: inbox)
_ = try await sub.connect()
let granted = try await sub.subscribe(topicFilter: "smoke/#", qos: .atLeastOnce)
must(granted.reasonCode < 0x80, "subscribe granted")

let pub = MqttClient(options: options(port, "swift-pub"))
_ = try await pub.connect()
try await pub.publish(topic: "smoke/swift", payload: Data("hello-swift".utf8), qos: .atLeastOnce, retain: false)

var received: MqttMessage?
for _ in 0..<50 {
    received = inbox.first()
    if received != nil { break }
    try await Task.sleep(nanoseconds: 100_000_000)
}
must(received != nil, "message delivered")
must(received!.topic == "smoke/swift" && String(decoding: received!.payload, as: UTF8.self) == "hello-swift", "message content")

var refused = false
do { _ = try await MqttClient(options: options(port, "swift-bad", user: "bad")).connect() } catch { refused = true }
must(refused, "auth provider must refuse bad credentials")
must(events.connected().isSuperset(of: ["swift-sub", "swift-pub"]), "event listener saw connections")

// Pull-style delivery (the path Dart/Haskell use) works from Swift too.
let polled = MqttClient(options: options(port, "swift-polled"))
polled.enableMessageQueue(capacity: 8)
_ = try await polled.connect()
_ = try await polled.subscribe(topicFilter: "smoke/polled", qos: .atLeastOnce)
try await pub.publish(topic: "smoke/polled", payload: Data("pulled".utf8), qos: .atLeastOnce, retain: false)
let pulled = await polled.nextMessage(timeoutMs: 5000)
must(pulled != nil && String(decoding: pulled!.payload, as: UTF8.self) == "pulled", "polled message")
try await polled.disconnect()

try await pub.disconnect()
try await sub.disconnect()
try await broker.stop()
must(!broker.isRunning(), "broker stopped")
print("SMOKE OK")
