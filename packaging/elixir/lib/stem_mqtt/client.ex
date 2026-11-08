defmodule StemMqtt.Client do
  @moduledoc """
  An MQTT 3.1.1 / 5.0 client.

  The process that creates a client owns it: received messages and connection
  losses arrive in its mailbox, and when it exits the client disconnects.

      {:ok, client} = StemMqtt.Client.new(host: "localhost", client_id: "sensor-1")
      {:ok, %{session_present: false}} = StemMqtt.Client.connect(client)
      {:ok, _reason_code} = StemMqtt.Client.subscribe(client, "sensors/#", qos: 1)
      :ok = StemMqtt.Client.publish(client, "sensors/t", "21.5", qos: 1)

      receive do
        {:stem_mqtt, id, {:message, %StemMqtt.Message{topic: topic, payload: payload}}}
        when id == client.id ->
          {topic, payload}

        {:stem_mqtt, id, {:disconnected, reason}} when id == client.id ->
          reason
      end

  ## Options

    * `:host` (required), `:port` (1883), `:client_id` (`""`: the broker assigns one)
    * `:version` - `:v5` (default) or `:v311`
    * `:clean_start` (`true`), `:keep_alive_secs` (30)
    * `:username`, `:password` (binary)
    * `:will` - `%{topic: ..., payload: ..., qos: 0..2, retain: false}`
    * `:connect_timeout_secs` (10), `:operation_timeout_secs` (0: the default)
    * `:auto_reconnect` (`false`), `:reconnect_backoff_secs`, `:reconnect_max_backoff_secs`
    * `:tls` - `%{ca_cert_pem: ..., client_cert_pem: ..., client_key_pem: ...,
      insecure_skip_certificate_verification: false}`; `%{}` uses the system roots
    * `:max_packet_size` (0: the default)

  Unknown options raise `ArgumentError`.

  ## Errors

  Operations return `{:error, {kind, message}}` with `kind` one of
  `:malformed_packet`, `:protocol`, `:io`, `:connection_refused`,
  `:not_connected`, `:packet_too_large`, `:already_connected`, `:timeout`,
  `:session`.
  """

  alias StemMqtt.Native

  @enforce_keys [:ref, :id]
  defstruct [:ref, :id]

  @typedoc "`id` tags every message this client sends to its owner."
  @type t :: %__MODULE__{ref: reference(), id: pos_integer()}
  @type error :: {:error, {atom(), String.t()}}

  @doc "Creates a client owned by `owner` (default: the caller). Does not connect."
  @spec new(keyword() | map(), pid()) :: {:ok, t()}
  def new(options, owner \\ self()) do
    {ref, id} = Native.client_new(Map.new(options), owner)
    {:ok, %__MODULE__{ref: ref, id: id}}
  end

  @spec connect(t()) :: {:ok, %{session_present: boolean(), reason_code: 0..255}} | error()
  def connect(%__MODULE__{ref: ref}), do: Native.await(&Native.client_connect(ref, &1))

  @doc "Options: `:qos` (0), `:retain` (`false`)."
  @spec publish(t(), String.t(), iodata(), keyword()) :: :ok | error()
  def publish(%__MODULE__{ref: ref}, topic, payload, opts \\ []) do
    qos = Keyword.get(opts, :qos, 0)
    retain = Keyword.get(opts, :retain, false)
    payload = IO.iodata_to_binary(payload)
    Native.await(&Native.client_publish(ref, &1, topic, payload, qos, retain))
  end

  @doc "Options: `:qos` (0). Returns the broker's reason code (the granted QoS on success)."
  @spec subscribe(t(), String.t(), keyword()) :: {:ok, 0..255} | error()
  def subscribe(%__MODULE__{ref: ref}, filter, opts \\ []) do
    qos = Keyword.get(opts, :qos, 0)
    Native.await(&Native.client_subscribe(ref, &1, filter, qos))
  end

  @spec unsubscribe(t(), String.t()) :: :ok | error()
  def unsubscribe(%__MODULE__{ref: ref}, filter),
    do: Native.await(&Native.client_unsubscribe(ref, &1, filter))

  @spec disconnect(t()) :: :ok | error()
  def disconnect(%__MODULE__{ref: ref}), do: Native.await(&Native.client_disconnect(ref, &1))

  @spec connected?(t()) :: boolean()
  def connected?(%__MODULE__{ref: ref}), do: Native.client_is_connected(ref)

  @doc "Why the last connection ended, or `nil`."
  @spec last_disconnect_reason(t()) :: String.t() | nil
  def last_disconnect_reason(%__MODULE__{ref: ref}), do: Native.client_last_disconnect_reason(ref)
end
