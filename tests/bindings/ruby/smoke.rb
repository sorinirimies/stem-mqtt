# Runtime smoke test for the Ruby gems: client + broker, both callback
# directions, real sockets. Prints `SMOKE OK` on success (the runner checks it).
#
# Scenario (identical in every language's smoke test):
#   1. start a broker on an ephemeral port with an auth provider + event listener
#   2. a subscriber connects and subscribes (QoS 1)
#   3. a publisher connects and publishes a QoS 1 message
#   4. the subscriber's listener receives it (broker -> client callback path)
#   5. a client with bad credentials is refused (foreign auth callback path)
#   6. the broker's event listener saw the connections (foreign event callback path)
#   7. broker stops cleanly
#
# The runner installs the stem-mqtt-client and stem-mqtt-broker gems built for
# this platform, so this also proves the gems load their bundled library.

require "timeout"
require "stem_mqtt_client"
require "stem_mqtt_broker"

C = MqttClient
B = MqttBroker

def check(cond, what)
  raise "smoke check failed: #{what}" unless cond
end

# Callback interfaces are duck-typed: any object with the methods will do.
class Inbox
  attr_reader :messages

  def initialize
    @messages = Queue.new
  end

  def on_message(message) = @messages << message
  def on_disconnected(_reason) = nil
end

class Auth
  def authenticate(_client_id, username, _password) = username != "bad"
end

class Events
  attr_reader :connected

  def initialize
    @connected = []
  end

  def on_client_connected(client_id) = @connected << client_id
  def on_client_disconnected(_client_id, _reason) = nil
  def on_message_published(_client_id, _topic, _qos) = nil
end

# MQTT 5 enhanced auth: challenge with [7]; the response must be [8].
class AddOneAuth
  def step(_client_id, _method, data, round)
    return B::EnhancedAuthStep.new(outcome: B::EnhancedAuthOutcome::CONTINUE, data: "\x07".b) if round.zero?

    ok = !data.nil? && data.b == "\x08".b
    B::EnhancedAuthStep.new(
      outcome: ok ? B::EnhancedAuthOutcome::SUCCESS : B::EnhancedAuthOutcome::FAILURE,
      data: ok ? "welcome".b : "".b
    )
  end
end

class AddOneHandler
  def respond(_method, challenge) = challenge.bytes.map { |b| b + 1 }.pack("C*")
end

def options(port, client_id, username: nil, **extra)
  C::ConnectOptions.new(
    host: "127.0.0.1", port: port, client_id: client_id, version: C::MqttVersion::V5,
    clean_start: true, keep_alive_secs: 30, username: username, password: nil, will: nil,
    connect_timeout_secs: 10, operation_timeout_secs: 5, auto_reconnect: false,
    reconnect_backoff_secs: 0, reconnect_max_backoff_secs: 0, tls: nil, **extra
  )
end

# max_packet_size / max_outbound_queue are omitted on purpose: they have record
# defaults, which proves default-argument support in this binding.
config = B::MqttBrokerConfig.new(
  bind_address: "127.0.0.1", port: 0, ws_port: nil, allow_anonymous: true, max_clients: 0,
  max_qos: B::QoS::EXACTLY_ONCE, max_retained_messages: 100, max_queued_per_client: 100,
  redelivery_interval_secs: 0, tls: nil
)
broker = B::MqttBroker.new(config)
events = Events.new
broker.set_auth_provider(Auth.new)
broker.set_event_listener(events)
broker.start
check(broker.is_running, "broker running")
port = broker.bound_port
check(port, "broker must report its bound port")

inbox = Inbox.new
sub = C::MqttClient.new(options(port, "rb-sub"))
sub.set_message_listener(inbox)
sub.connect
result = sub.subscribe("smoke/#", C::QoS::AT_LEAST_ONCE)
check(result.reason_code < 0x80, "subscribe accepted: #{result.reason_code}")

pub = C::MqttClient.new(options(port, "rb-pub"))
pub.connect
pub.publish("smoke/rb", "hello-ruby".b, C::QoS::AT_LEAST_ONCE, false)

msg = Timeout.timeout(5) { inbox.messages.pop }
check(msg.topic == "smoke/rb" && msg.payload.b == "hello-ruby".b, "delivered #{msg.inspect}")

refused =
  begin
    C::MqttClient.new(options(port, "rb-bad", username: "bad")).connect
    false
  rescue StandardError
    true
  end
check(refused, "auth provider callback must be able to refuse a client")

check((%w[rb-sub rb-pub] - events.connected).empty?, "events saw #{events.connected}")

# Pull-style delivery (the path Dart/Haskell use), alongside callbacks.
polled = C::MqttClient.new(options(port, "rb-polled"))
polled.enable_message_queue(8)
polled.connect
polled.subscribe("smoke/polled", C::QoS::AT_LEAST_ONCE)
pub.publish("smoke/polled", "pulled".b, C::QoS::AT_LEAST_ONCE, false)
pulled = polled.next_message(5000)
check(pulled && pulled.payload.b == "pulled".b, "pulled #{pulled.inspect}")
check(polled.next_message(50).nil?, "queue drained")
polled.disconnect

# MQTT 5 enhanced authentication: challenge/response through foreign callbacks
# in BOTH directions (broker provider + client handler).
broker.set_enhanced_auth_provider(AddOneAuth.new)
eopts = options(port, "rb-eauth", auth_method: "X-ADD-ONE", auth_data: "hello".b)
eclient = C::MqttClient.new(eopts)
eclient.set_auth_handler(AddOneHandler.new)
eclient.connect
check(eclient.is_connected, "enhanced auth connected")
eclient.disconnect

pub.disconnect
sub.disconnect
broker.stop
check(!broker.is_running, "broker stopped")
puts "SMOKE OK"
