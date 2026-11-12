defmodule StemMqtt.MixProject do
  use Mix.Project

  # Bumped with the workspace by scripts/bump_version.nu.
  @version "0.8.3"
  @source_url "https://github.com/sorinirimies/stem-mqtt"

  def project do
    [
      app: :stem_mqtt,
      version: @version,
      elixir: "~> 1.15",
      start_permanent: Mix.env() == :prod,
      deps: deps(),
      description:
        "MQTT 3.1.1/5.0 client and embeddable broker for Elixir and Erlang, in Rust (precompiled NIF)",
      package: package(),
      docs: docs(),
      source_url: @source_url
    ]
  end

  def application, do: [extra_applications: [:logger]]

  defp deps do
    [
      {:rustler_precompiled, "~> 0.8"},
      # Only to build the NIF from source (STEM_MQTT_BUILD=1, in this repo).
      {:rustler, ">= 0.36.0", optional: true},
      {:ex_doc, "~> 0.34", only: :dev, runtime: false}
    ]
  end

  defp package do
    [
      licenses: ["MIT"],
      links: %{"GitHub" => @source_url},
      files: ~w(lib src mix.exs README.md checksum-Elixir.StemMqtt.Native.exs)
    ]
  end

  defp docs do
    [main: "readme", extras: ["README.md"], source_ref: "v#{@version}"]
  end
end
