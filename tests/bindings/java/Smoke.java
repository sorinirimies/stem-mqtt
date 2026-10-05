// Runtime smoke test for the Java bindings: client + broker, both callback
// directions, real sockets. Prints "SMOKE OK" on success.
// Scenario is identical in every language's smoke test (see tests/bindings/python/smoke.py).
// Needs JDK 22+ (Foreign Function & Memory API) and --enable-native-access.
import java.nio.charset.StandardCharsets;
import java.util.Set;
import java.util.concurrent.*;
import uniffi.mqtt_broker.*;
import uniffi.mqtt_client.*;

public class Smoke {
    static void must(boolean cond, String what) {
        if (!cond) {
            System.err.println("FAIL " + what);
            System.exit(1);
        }
    }

    static ConnectOptions options(short port, String id, String user) {
        return new ConnectOptions("127.0.0.1", port, id, MqttVersion.V5, true, (short) 30, user, null, null,
                10, 5, false, 0, 0, null, 0, null, null);
    }

    public static void main(String[] args) throws Exception {
        BlockingQueue<MqttMessage> inbox = new LinkedBlockingQueue<>();
        Set<String> connected = ConcurrentHashMap.newKeySet();

        MqttBroker broker = new MqttBroker(new MqttBrokerConfig("127.0.0.1", (short) 0, null, true, 0,
                uniffi.mqtt_broker.QoS.EXACTLY_ONCE, 100, 100, 0, null, 0, 0, 0));
        broker.setAuthProvider((clientId, username, password) -> !"bad".equals(username));
        broker.setEventListener(new MqttBrokerEventListener() {
            public void onClientConnected(String clientId) { connected.add(clientId); }
            public void onClientDisconnected(String clientId, String reason) { }
            public void onMessagePublished(String clientId, String topic, uniffi.mqtt_broker.QoS qos) { }
        });
        broker.start().get(10, TimeUnit.SECONDS);
        must(broker.isRunning(), "broker running");
        Short boundPort = broker.boundPort();
        must(boundPort != null && boundPort != 0, "bound port");
        short port = boundPort;

        MqttClient sub = new MqttClient(options(port, "java-sub", null));
        sub.setMessageListener(new MqttMessageListener() {
            public void onMessage(MqttMessage message) { inbox.add(message); }
            public void onDisconnected(String reason) { }
        });
        sub.connect().get(10, TimeUnit.SECONDS);
        SubscribeResult granted = sub.subscribe("smoke/#", uniffi.mqtt_client.QoS.AT_LEAST_ONCE).get(10, TimeUnit.SECONDS);
        must(granted.reasonCode() >= 0 && granted.reasonCode() < 0x80, "subscribe granted");

        MqttClient pub = new MqttClient(options(port, "java-pub", null));
        pub.connect().get(10, TimeUnit.SECONDS);
        pub.publish("smoke/java", "hello-java".getBytes(StandardCharsets.UTF_8), uniffi.mqtt_client.QoS.AT_LEAST_ONCE, false)
                .get(10, TimeUnit.SECONDS);

        MqttMessage message = inbox.poll(5, TimeUnit.SECONDS);
        must(message != null, "message delivered");
        must(message.topic().equals("smoke/java")
                && new String(message.payload(), StandardCharsets.UTF_8).equals("hello-java"), "message content");

        boolean refused = false;
        try {
            new MqttClient(options(port, "java-bad", "bad")).connect().get(10, TimeUnit.SECONDS);
        } catch (ExecutionException e) {
            refused = true;
        }
        must(refused, "auth provider must refuse bad credentials");

        must(connected.contains("java-sub") && connected.contains("java-pub"), "event listener saw connections");

        pub.disconnect().get(10, TimeUnit.SECONDS);
        sub.disconnect().get(10, TimeUnit.SECONDS);
        broker.stop().get(10, TimeUnit.SECONDS);
        must(!broker.isRunning(), "broker stopped");
        System.out.println("SMOKE OK");
        System.exit(0); // the FFM cleaner threads are non-daemon in some JDKs
    }
}
