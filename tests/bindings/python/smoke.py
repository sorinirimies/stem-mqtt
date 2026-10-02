"""Runtime smoke test for the Python bindings: client + broker, both callback
directions, real sockets. Prints `SMOKE OK` on success (the runner checks it).

Scenario (identical in every language's smoke test):
  1. start a broker on an ephemeral port with an auth provider + event listener
  2. a subscriber connects and subscribes (QoS 1)
  3. a publisher connects and publishes a QoS 1 message
  4. the subscriber's listener receives it (broker -> client callback path)
  5. a client with bad credentials is refused (foreign auth callback path)
  6. the broker's event listener saw the connections (foreign event callback path)
  7. broker stops cleanly
"""

import asyncio
import threading

# The runner stages the generated modules (they use package-relative imports)
# plus the native libraries into the `mqttpkg` package.
from mqttpkg import mqtt_broker, mqtt_client


class Inbox(mqtt_client.MqttMessageListener):
    def __init__(self):
        self.messages = []
        self.event = threading.Event()

    def on_message(self, message):
        self.messages.append(message)
        self.event.set()

    def on_disconnected(self, reason):
        pass


class Auth(mqtt_broker.MqttAuthProvider):
    def authenticate(self, client_id, username, password):
        return username != "bad"


class Events(mqtt_broker.MqttBrokerEventListener):
    def __init__(self):
        self.connected = []

    def on_client_connected(self, client_id):
        self.connected.append(client_id)

    def on_client_disconnected(self, client_id, reason):
        pass

    def on_message_published(self, client_id, topic, qos):
        pass


def options(port, client_id, username=None):
    return mqtt_client.ConnectOptions(
        host="127.0.0.1",
        port=port,
        client_id=client_id,
        version=mqtt_client.MqttVersion.V5,
        clean_start=True,
        keep_alive_secs=30,
        username=username,
        password=None,
        will=None,
        connect_timeout_secs=10,
        operation_timeout_secs=5,
        auto_reconnect=False,
        reconnect_backoff_secs=0,
        reconnect_max_backoff_secs=0,
        tls=None,
    )


async def main():
    # max_packet_size / max_outbound_queue are omitted on purpose: they have
    # record defaults, which proves default-argument support in this binding.
    config = mqtt_broker.MqttBrokerConfig(
        bind_address="127.0.0.1",
        port=0,
        ws_port=None,
        allow_anonymous=True,
        max_clients=0,
        max_qos=mqtt_client.QoS.EXACTLY_ONCE,
        max_retained_messages=100,
        max_queued_per_client=100,
        redelivery_interval_secs=0,
        tls=None,
    )
    broker = mqtt_broker.MqttBroker(config)
    events = Events()
    broker.set_auth_provider(Auth())
    broker.set_event_listener(events)
    await broker.start()
    assert broker.is_running()
    port = broker.bound_port()
    assert port, "broker must report its bound port"

    inbox = Inbox()
    sub = mqtt_client.MqttClient(options(port, "py-sub"))
    sub.set_message_listener(inbox)
    await sub.connect()
    result = await sub.subscribe("smoke/#", mqtt_client.QoS.AT_LEAST_ONCE)
    assert result.reason_code < 0x80, result

    pub = mqtt_client.MqttClient(options(port, "py-pub"))
    await pub.connect()
    await pub.publish("smoke/py", b"hello-python", mqtt_client.QoS.AT_LEAST_ONCE, False)

    assert await asyncio.to_thread(inbox.event.wait, 5), "no message delivered"
    msg = inbox.messages[0]
    assert msg.topic == "smoke/py" and bytes(msg.payload) == b"hello-python", msg

    refused = False
    try:
        await mqtt_client.MqttClient(options(port, "py-bad", username="bad")).connect()
    except Exception:
        refused = True
    assert refused, "auth provider callback must be able to refuse a client"

    assert {"py-sub", "py-pub"} <= set(events.connected), events.connected

    await pub.disconnect()
    await sub.disconnect()
    await broker.stop()
    assert not broker.is_running()
    print("SMOKE OK")


asyncio.run(main())
