%% Erlang API for stem_mqtt: the same client and broker as the Elixir modules.
%% Options are maps with atom keys and binary strings, e.g.
%%   {ok, B} = stem_mqtt:broker(#{port => 0}), ok = stem_mqtt:start(B),
%%   {ok, C} = stem_mqtt:client(#{host => <<"127.0.0.1">>, port => stem_mqtt:port(B)}),
%%   {ok, _} = stem_mqtt:connect(C).
%% Messages arrive as {stem_mqtt, Id, Event}, Id = stem_mqtt:id(Handle).
-module(stem_mqtt).

-export([client/1, connect/1, publish/4, subscribe/3, unsubscribe/2, disconnect/1,
         is_connected/1, broker/1, start/1, stop/1, is_running/1, port/1,
         resolve_auth/3, id/1]).

-define(CLIENT, 'Elixir.StemMqtt.Client').
-define(BROKER, 'Elixir.StemMqtt.Broker').

client(Options) -> ?CLIENT:new(Options).
connect(Client) -> ?CLIENT:connect(Client).
publish(Client, Topic, Payload, Opts) -> ?CLIENT:publish(Client, Topic, Payload, maps:to_list(Opts)).
subscribe(Client, Filter, Opts) -> ?CLIENT:subscribe(Client, Filter, maps:to_list(Opts)).
unsubscribe(Client, Filter) -> ?CLIENT:unsubscribe(Client, Filter).
disconnect(Client) -> ?CLIENT:disconnect(Client).
is_connected(Client) -> ?CLIENT:'connected?'(Client).

broker(Config) -> ?BROKER:new(Config).
start(Broker) -> ?BROKER:start(Broker).
stop(Broker) -> ?BROKER:stop(Broker).
is_running(Broker) -> ?BROKER:'running?'(Broker).
port(Broker) -> ?BROKER:port(Broker).
resolve_auth(Broker, Id, Allow) -> ?BROKER:resolve_auth(Broker, Id, Allow).

%% The id that tags a handle's {stem_mqtt, Id, Event} messages.
id(#{id := Id}) -> Id.
