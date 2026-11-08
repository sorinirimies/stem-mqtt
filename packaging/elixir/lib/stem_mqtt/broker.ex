defmodule StemMqtt.Broker do
  @moduledoc """
  An embeddable MQTT 3.1.1 / 5.0 broker: for tests, edge devices, or an app
  that is its own broker.

  The process that creates a broker owns it; when it exits the broker stops.

      {:ok, broker} = StemMqtt.Broker.new(port: 0, events: true)
      :ok = StemMqtt.Broker.start(broker)
      port = StemMqtt.Broker.port(broker)

  ## Config

    * `:bind_address` (`"127.0.0.1"`), `:port` (1883; 0 picks a free port)
    * `:ws_port` - also serve MQTT over WebSocket
    * `:tls` - `%{port: ..., cert_pem: ..., key_pem: ..., client_ca_pem: nil}`
    * `:allow_anonymous` (`true`), `:max_clients` (0: unlimited), `:max_qos` (2)
    * `:max_retained_messages`, `:max_queued_per_client`, `:redelivery_interval_secs`,
      `:max_packet_size`, `:max_outbound_queue`, `:session_expiry_secs`

  Owner options, not passed to the broker:

    * `:events` (`false`) - send `{:stem_mqtt, broker.id, event}` for
      `{:client_connected, client_id}`, `{:client_disconnected, client_id, reason}`
      and `{:message_published, client_id, topic, qos}`
    * `:auth` (`false`) - send every CONNECT as
      `{:stem_mqtt, broker.id, {:auth_request, id, client_id, username, password}}`;
      answer with `resolve_auth/3` within 10 seconds or it is denied
    * `:owner` (the caller)

  Unknown keys raise `ArgumentError`. Errors are `{:error, {kind, message}}`
  with `kind` one of `:already_running`, `:io`, `:tls`.
  """

  alias StemMqtt.Native

  @enforce_keys [:ref, :id]
  defstruct [:ref, :id]

  @type t :: %__MODULE__{ref: reference(), id: pos_integer()}
  @type error :: {:error, {atom(), String.t()}}

  @doc "Creates a stopped broker."
  @spec new(keyword() | map()) :: {:ok, t()}
  def new(config \\ []) do
    {owner_opts, config} = Map.split(Map.new(config), [:events, :auth, :owner])
    owner = Map.get(owner_opts, :owner, self())
    events = Map.get(owner_opts, :events, false)
    auth = Map.get(owner_opts, :auth, false)
    {ref, id} = Native.broker_new(config, owner, events, auth)
    {:ok, %__MODULE__{ref: ref, id: id}}
  end

  @doc "Binds every listener and starts accepting connections."
  @spec start(t()) :: :ok | error()
  def start(%__MODULE__{ref: ref}), do: Native.await(&Native.broker_start(ref, &1))

  @spec stop(t()) :: :ok | error()
  def stop(%__MODULE__{ref: ref}), do: Native.await(&Native.broker_stop(ref, &1))

  @spec running?(t()) :: boolean()
  def running?(%__MODULE__{ref: ref}), do: Native.broker_is_running(ref)

  @spec client_count(t()) :: non_neg_integer()
  def client_count(%__MODULE__{ref: ref}), do: Native.broker_client_count(ref)

  @doc "The TCP port the broker listens on (useful with `port: 0`), or `nil` when stopped."
  @spec port(t()) :: :inet.port_number() | nil
  def port(%__MODULE__{ref: ref}), do: Native.broker_bound_port(ref)

  @spec ws_port(t()) :: :inet.port_number() | nil
  def ws_port(%__MODULE__{ref: ref}), do: Native.broker_bound_ws_port(ref)

  @spec tls_port(t()) :: :inet.port_number() | nil
  def tls_port(%__MODULE__{ref: ref}), do: Native.broker_bound_tls_port(ref)

  @doc "Admits or refuses the CONNECT behind an `:auth_request`; `false` if it already expired."
  @spec resolve_auth(t(), non_neg_integer(), boolean()) :: boolean()
  def resolve_auth(%__MODULE__{ref: ref}, id, allow),
    do: Native.broker_resolve_auth(ref, id, allow)
end
