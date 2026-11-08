defmodule StemMqttTest do
  use ExUnit.Case, async: true

  alias StemMqtt.{Broker, Client, Message}

  defp started_broker(config \\ []) do
    {:ok, broker} = Broker.new(Keyword.merge([port: 0], config))
    :ok = Broker.start(broker)
    on_exit(fn -> Broker.stop(broker) end)
    broker
  end

  defp connected_client(broker, client_id) do
    {:ok, client} = Client.new(host: "127.0.0.1", port: Broker.port(broker), client_id: client_id)
    {:ok, _} = Client.connect(client)
    client
  end

  test "a published message reaches a subscriber's owner as a Message struct" do
    broker = started_broker()
    sub = connected_client(broker, "sub")
    pub = connected_client(broker, "pub")

    assert {:ok, 1} = Client.subscribe(sub, "sensors/#", qos: 1)
    assert :ok = Client.publish(pub, "sensors/t", ["21", ".5"], qos: 1)

    id = sub.id

    assert_receive {:stem_mqtt, ^id,
                    {:message,
                     %Message{topic: "sensors/t", payload: "21.5", qos: 1, retain: false}}},
                   2_000
  end

  test "binary payloads arrive unchanged" do
    broker = started_broker()
    client = connected_client(broker, "bin")
    payload = :crypto.strong_rand_bytes(4096) <> <<0, 255>>
    {:ok, _} = Client.subscribe(client, "bin", qos: 0)
    :ok = Client.publish(client, "bin", payload)
    id = client.id
    assert_receive {:stem_mqtt, ^id, {:message, %Message{payload: ^payload}}}, 2_000
  end

  test "connect reports session state; disconnect and connected? agree" do
    broker = started_broker()
    {:ok, client} = Client.new(host: "127.0.0.1", port: Broker.port(broker), client_id: "c")
    refute Client.connected?(client)
    assert {:ok, %{session_present: false, reason_code: 0}} = Client.connect(client)
    assert Client.connected?(client)
    assert {:error, {:already_connected, _}} = Client.connect(client)
    assert :ok = Client.disconnect(client)
    refute Client.connected?(client)
  end

  test "operations on a client that never connected fail with :not_connected" do
    {:ok, client} = Client.new(host: "127.0.0.1", port: 1)
    assert {:error, {:not_connected, "not connected"}} = Client.publish(client, "t", "x")
  end

  test "an unreachable broker is an :io error, not a crash" do
    {:ok, client} = Client.new(host: "127.0.0.1", port: 1, connect_timeout_secs: 1)
    assert {:error, {:io, _}} = Client.connect(client)
  end

  test "invalid topics are rejected before reaching the network" do
    broker = started_broker()
    client = connected_client(broker, "bad-topic")
    assert {:error, {:protocol, message}} = Client.publish(client, "a/+", "x")
    assert message =~ "invalid topic name"
  end

  test "unknown or mistyped options raise ArgumentError" do
    assert_raise ArgumentError, "unknown client option :hots", fn -> Client.new(hots: "x") end

    assert_raise ArgumentError, "client option :host is required", fn ->
      Client.new(port: 1883)
    end

    assert_raise ArgumentError, "client.port must be a port number", fn ->
      Client.new(host: "h", port: 70_000)
    end

    assert_raise ArgumentError, "qos must be 0, 1 or 2", fn ->
      {:ok, c} = Client.new(host: "h")
      Client.publish(c, "t", "x", qos: 3)
    end

    assert_raise ArgumentError, "unknown broker option :prot", fn -> Broker.new(prot: 1) end
  end

  test "broker events reach the owner" do
    broker = started_broker(events: true)
    client = connected_client(broker, "evented")
    id = broker.id
    assert_receive {:stem_mqtt, ^id, {:client_connected, "evented"}}, 2_000
    :ok = Client.publish(client, "e", "x", qos: 1)
    assert_receive {:stem_mqtt, ^id, {:message_published, "evented", "e", 1}}, 2_000
    :ok = Client.disconnect(client)
    assert_receive {:stem_mqtt, ^id, {:client_disconnected, "evented", _reason}}, 2_000
  end

  test "auth requests go to the owner, who admits or refuses each CONNECT" do
    broker = started_broker(auth: true)
    port = Broker.port(broker)
    id = broker.id
    parent = self()

    connect = fn client_id, password ->
      Task.async(fn ->
        {:ok, c} =
          Client.new(
            host: "127.0.0.1",
            port: port,
            client_id: client_id,
            username: "u",
            password: password
          )

        send(parent, {:connecting, client_id})
        Client.connect(c)
      end)
    end

    good = connect.("good", "secret")
    assert_receive {:stem_mqtt, ^id, {:auth_request, req, "good", "u", "secret"}}, 2_000
    assert Broker.resolve_auth(broker, req, true)
    assert {:ok, _} = Task.await(good)

    bad = connect.("bad", "wrong")
    assert_receive {:stem_mqtt, ^id, {:auth_request, req, "bad", "u", "wrong"}}, 2_000
    assert Broker.resolve_auth(broker, req, false)
    assert {:error, {:connection_refused, _}} = Task.await(bad)
    refute Broker.resolve_auth(broker, req, true), "an answered request is gone"
  end

  test "a broker stops when its owner exits" do
    parent = self()

    owner =
      spawn(fn ->
        {:ok, broker} = Broker.new(port: 0)
        :ok = Broker.start(broker)
        send(parent, {:broker, broker})
        receive do: (:exit -> :ok)
      end)

    assert_receive {:broker, broker}, 2_000
    assert Broker.running?(broker)
    send(owner, :exit)
    wait_until(fn -> not Broker.running?(broker) end)
  end

  test "starting twice is :already_running; stop frees the port" do
    broker = started_broker()
    assert {:error, {:already_running, _}} = Broker.start(broker)
    assert :ok = Broker.stop(broker)
    refute Broker.running?(broker)
    assert Broker.port(broker) == nil
  end

  test "the Erlang API drives the same client and broker" do
    {:ok, b} = :stem_mqtt.broker(%{port: 0})
    :ok = :stem_mqtt.start(b)
    {:ok, c} = :stem_mqtt.client(%{host: "127.0.0.1", port: :stem_mqtt.port(b), client_id: "erl"})
    {:ok, _} = :stem_mqtt.connect(c)
    {:ok, 0} = :stem_mqtt.subscribe(c, "erl/t", %{})
    :ok = :stem_mqtt.publish(c, "erl/t", "hi", %{})
    id = :stem_mqtt.id(c)
    assert_receive {:stem_mqtt, ^id, {:message, %{payload: "hi"}}}, 2_000
    :ok = :stem_mqtt.stop(b)
  end

  defp wait_until(fun, attempts \\ 50) do
    cond do
      fun.() -> :ok
      attempts == 0 -> flunk("condition never became true")
      true -> Process.sleep(50) && wait_until(fun, attempts - 1)
    end
  end
end
