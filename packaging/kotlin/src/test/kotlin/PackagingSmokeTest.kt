import kotlinx.coroutines.runBlocking
import kotlin.test.Test
import kotlin.test.assertFalse
import kotlin.test.assertNotNull
import kotlin.test.assertTrue
import uniffi.mqtt_broker.MqttBroker
import uniffi.mqtt_broker.MqttBrokerConfig
import uniffi.mqtt_broker.QoS

class PackagingSmokeTest {
    @Test
    fun `packaged broker native library starts and stops`() = runBlocking {
        val config = MqttBrokerConfig(
            bindAddress = "127.0.0.1",
            port = 0u,
            wsPort = null,
            allowAnonymous = true,
            maxClients = 0u,
            maxQos = QoS.EXACTLY_ONCE,
            maxRetainedMessages = 100u,
            maxQueuedPerClient = 100u,
            redeliveryIntervalSecs = 0u,
            tls = null,
        )

        MqttBroker(config).use { broker ->
            assertFalse(broker.isRunning())
            broker.start()
            assertTrue(broker.isRunning())
            assertNotNull(broker.boundPort())
            broker.stop()
            assertFalse(broker.isRunning())
        }
    }
}
