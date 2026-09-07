"""Runtime smoke test for installed stem-mqtt client + broker wheels."""

import asyncio

import mqtt_broker
import mqtt_client


async def main() -> None:
    # External UniFFI enums in the broker wheel are generated in the
    # mqtt_broker module; use that module's QoS when constructing broker data.
    config = mqtt_broker.MqttBrokerConfig(
        bind_address="127.0.0.1",
        port=0,
        ws_port=None,
        allow_anonymous=True,
        max_clients=0,
        max_qos=mqtt_broker.QoS.EXACTLY_ONCE,
        max_retained_messages=100,
        max_queued_per_client=100,
        redelivery_interval_secs=0,
        tls=None,
    )
    broker = mqtt_broker.MqttBroker(config)
    await broker.start()
    assert broker.is_running()

    port = broker.bound_port()
    assert port is not None
    options = mqtt_client.ConnectOptions(
        host="127.0.0.1",
        port=port,
        client_id="python-wheel-smoke",
        version=mqtt_client.MqttVersion.V5,
        clean_start=True,
        keep_alive_secs=30,
        username=None,
        password=None,
        will=None,
        connect_timeout_secs=10,
        operation_timeout_secs=5,
        auto_reconnect=False,
        reconnect_backoff_secs=0,
        reconnect_max_backoff_secs=0,
        tls=None,
    )
    client = mqtt_client.MqttClient(options)
    await client.connect()
    assert client.is_connected()
    await client.disconnect()
    await broker.stop()
    assert not broker.is_running()


if __name__ == "__main__":
    asyncio.run(main())
    print("Python client+broker wheel smoke test passed")
