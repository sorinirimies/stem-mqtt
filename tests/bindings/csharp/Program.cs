// Runtime smoke test for the C# bindings: client + broker, both callback
// directions, real sockets. Prints "SMOKE OK" on success.
// Scenario is identical in every language's smoke test (see tests/bindings/python/smoke.py).
using uniffi.mqtt_broker;
using uniffi.mqtt_client;

static void Must(bool cond, string what)
{
    if (!cond) { Console.Error.WriteLine($"FAIL {what}"); Environment.Exit(1); }
}

static ConnectOptions Options(ushort port, string id, string? user = null) => new(
    Host: "127.0.0.1", Port: port, ClientId: id, Version: MqttVersion.V5, CleanStart: true,
    KeepAliveSecs: 30, Username: user, Password: null, Will: null, ConnectTimeoutSecs: 10,
    OperationTimeoutSecs: 5, AutoReconnect: false, ReconnectBackoffSecs: 0,
    ReconnectMaxBackoffSecs: 0, Tls: null);

var inbox = new Inbox();
var events = new Events();

var broker = new MqttBroker(new MqttBrokerConfig(
    BindAddress: "127.0.0.1", Port: 0, WsPort: null, AllowAnonymous: true, MaxClients: 0,
    MaxQos: QoS.ExactlyOnce, MaxRetainedMessages: 100, MaxQueuedPerClient: 100,
    RedeliveryIntervalSecs: 0, Tls: null));
broker.SetAuthProvider(new Auth());
broker.SetEventListener(events);
await broker.Start();
Must(broker.IsRunning(), "broker running");
var port = broker.BoundPort() ?? 0;
Must(port != 0, "bound port");

var sub = new MqttClient(Options(port, "cs-sub"));
sub.SetMessageListener(inbox);
await sub.Connect();
var granted = await sub.Subscribe("smoke/#", QoS.AtLeastOnce);
Must(granted.ReasonCode < 0x80, "subscribe granted");

var pub = new MqttClient(Options(port, "cs-pub"));
await pub.Connect();
await pub.Publish("smoke/cs", System.Text.Encoding.UTF8.GetBytes("hello-csharp"), QoS.AtLeastOnce, false);

Must(inbox.Received.Wait(TimeSpan.FromSeconds(5)), "message delivered");
Must(inbox.Message!.Topic == "smoke/cs" &&
     System.Text.Encoding.UTF8.GetString(inbox.Message.Payload) == "hello-csharp", "message content");

var refused = false;
try { await new MqttClient(Options(port, "cs-bad", "bad")).Connect(); }
catch (Exception) { refused = true; }
Must(refused, "auth provider must refuse bad credentials");

Must(events.Connected.Contains("cs-sub") && events.Connected.Contains("cs-pub"), "event listener saw connections");

await pub.Disconnect();
await sub.Disconnect();
await broker.Stop();
Must(!broker.IsRunning(), "broker stopped");
Console.WriteLine("SMOKE OK");

class Inbox : MqttMessageListener
{
    public readonly ManualResetEventSlim Received = new(false);
    public MqttMessage? Message;
    public void OnMessage(MqttMessage message) { Message = message; Received.Set(); }
    public void OnDisconnected(string reason) { }
}

class Auth : MqttAuthProvider
{
    public bool Authenticate(string clientId, string? username, byte[]? password) => username != "bad";
}

class Events : MqttBrokerEventListener
{
    public readonly System.Collections.Concurrent.ConcurrentBag<string> Connected = new();
    public void OnClientConnected(string clientId) => Connected.Add(clientId);
    public void OnClientDisconnected(string clientId, string reason) { }
    public void OnMessagePublished(string clientId, string topic, QoS qos) { }
}
