defmodule StemMqtt.Message do
  @moduledoc """
  A PUBLISH delivered to a subscription, sent to the client's owner as
  `{:stem_mqtt, client.id, {:message, %StemMqtt.Message{}}}`.
  """

  @type t :: %__MODULE__{topic: String.t(), payload: binary(), qos: 0..2, retain: boolean()}
  defstruct [:topic, :payload, :qos, :retain]
end
