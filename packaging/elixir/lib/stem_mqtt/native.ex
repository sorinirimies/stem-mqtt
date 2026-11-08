defmodule StemMqtt.Native do
  @moduledoc false
  # The NIF (crates/mqtt-beam). Hex users get a precompiled build from the
  # GitHub release of this version; STEM_MQTT_BUILD=1 builds it from source,
  # which only works inside the stem-mqtt repository.

  version = Mix.Project.config()[:version]

  use RustlerPrecompiled,
    otp_app: :stem_mqtt,
    crate: "stem_mqtt_nif",
    base_url: "https://github.com/sorinirimies/stem-mqtt/releases/download/v#{version}",
    force_build: System.get_env("STEM_MQTT_BUILD") in ["1", "true"],
    path: "../../crates/mqtt-beam",
    version: version,
    nif_versions: ["2.15"],
    # Must match the build-nif matrix in .github/workflows/release.yml.
    targets: ~w(
      aarch64-apple-darwin
      x86_64-apple-darwin
      aarch64-unknown-linux-gnu
      x86_64-unknown-linux-gnu
      x86_64-pc-windows-msvc
    )

  def client_new(_options, _owner), do: err()
  def client_connect(_client, _request), do: err()
  def client_publish(_client, _request, _topic, _payload, _qos, _retain), do: err()
  def client_subscribe(_client, _request, _filter, _qos), do: err()
  def client_unsubscribe(_client, _request, _filter), do: err()
  def client_disconnect(_client, _request), do: err()
  def client_is_connected(_client), do: err()
  def client_last_disconnect_reason(_client), do: err()

  def broker_new(_config, _owner, _events, _auth), do: err()
  def broker_start(_broker, _request), do: err()
  def broker_stop(_broker, _request), do: err()
  def broker_is_running(_broker), do: err()
  def broker_client_count(_broker), do: err()
  def broker_bound_port(_broker), do: err()
  def broker_bound_ws_port(_broker), do: err()
  def broker_bound_tls_port(_broker), do: err()
  def broker_resolve_auth(_broker, _id, _allow), do: err()

  defp err, do: :erlang.nif_error(:nif_not_loaded)

  @doc false
  # Runs an async NIF call and waits for its `{:stem_mqtt_reply, id, result}`.
  # Rust bounds every operation with its own timeout, so this never waits forever.
  def await(fun) do
    request = System.unique_integer([:positive, :monotonic])
    :ok = fun.(request)

    receive do
      {:stem_mqtt_reply, ^request, result} -> result
    end
  end
end
