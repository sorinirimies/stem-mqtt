-- Runtime smoke test for the Lua bindings: client + broker, both callback
-- directions, real sockets. Prints "SMOKE OK" on success.
--
-- Scenario (identical in every language's smoke test): start a broker with an
-- auth provider and an event listener; subscribe with one client; publish QoS 1
-- from another; expect delivery; expect a bad-credentials client to be refused;
-- expect the broker's event listener to have seen the connections; stop.
--
-- Callbacks Rust makes from its own threads run on the Lua thread: during a
-- blocking call into either module, or in `poll()` (both modules share one queue).
local mqtt_client = require("mqtt_client")
local mqtt_broker = require("mqtt_broker")

local function must(cond, what)
    if not cond then
        io.stderr:write("FAIL " .. what .. "\n")
        os.exit(1)
    end
end

-- Runs queued callbacks until `done()` or `seconds` pass.
local function poll_until(done, seconds)
    local deadline = os.time() + seconds
    while not done() and os.time() <= deadline do
        mqtt_client.poll()
    end
    return done()
end

local function options(port, id, user)
    return {
        host = "127.0.0.1", port = port, client_id = id,
        version = "V5", clean_start = true, keep_alive_secs = 30,
        username = user, password = nil, will = nil,
        connect_timeout_secs = 10, operation_timeout_secs = 5,
        auto_reconnect = false, reconnect_backoff_secs = 0, reconnect_max_backoff_secs = 0,
        tls = nil,
    }
end

local broker = mqtt_broker.MqttBroker.new({
    bind_address = "127.0.0.1", port = 0, ws_port = nil,
    allow_anonymous = true, max_clients = 0, max_qos = "ExactlyOnce",
    max_retained_messages = 100, max_queued_per_client = 100,
    redelivery_interval_secs = 0, tls = nil,
})
local connected = {}
-- Traits Lua may implement: a table of methods, wrapped with from_impl.
broker:set_auth_provider(mqtt_broker.MqttAuthProvider.from_impl({
    authenticate = function(_, _, username, _) return username ~= "bad" end,
}))
broker:set_event_listener(mqtt_broker.MqttBrokerEventListener.from_impl({
    on_client_connected = function(_, client_id) connected[client_id] = true end,
    on_client_disconnected = function() end,
    on_message_published = function() end,
}))
broker:start()
must(broker:is_running(), "broker running")
local port = broker:bound_port()
must(port ~= nil and port ~= 0, "bound port")

local received = nil
local sub = mqtt_client.MqttClient.new(options(port, "lua-sub"))
sub:set_message_listener(mqtt_client.MqttMessageListener.from_impl({
    on_message = function(_, message) received = message end,
    on_disconnected = function() end,
}))
sub:connect()
must(sub:subscribe("smoke/#", "AtLeastOnce").reason_code < 0x80, "subscribe granted")

local pub = mqtt_client.MqttClient.new(options(port, "lua-pub"))
pub:connect()
pub:publish("smoke/lua", "hello-lua", "AtLeastOnce", false)

must(poll_until(function() return received ~= nil end, 5), "message delivery timed out")
must(received.topic == "smoke/lua" and received.payload == "hello-lua", "message content")

local refused, err = pcall(function()
    mqtt_client.MqttClient.new(options(port, "lua-bad", "bad")):connect()
end)
must(not refused and type(err) == "table" and err.tag == "ConnectionRefused",
    "auth provider must refuse bad credentials: " .. tostring(err))

must(poll_until(function() return connected["lua-sub"] and connected["lua-pub"] end, 5),
    "event listener saw connections")

pub:disconnect()
sub:disconnect()
broker:stop()
must(not broker:is_running(), "broker stopped")
print("SMOKE OK")
