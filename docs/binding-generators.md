# Binding generators: notes

Details behind the generator table in the [README](../README.md#generating-foreign-language-bindings).

## ¹ Dart and Haskell: pull-style API

Dart: foreign callbacks cannot be invoked from Rust's own threads (the VM aborts). Haskell: callback
interfaces are exposed only as opaque handles. So `MqttMessageListener`, `MqttAuthProvider`,
`MqttBrokerEventListener` and the enhanced-auth callbacks can't be implemented there — use the
**pull-style API** that exists for exactly this: `enable_message_queue` / `next_message(timeout_ms)` on the
client and `enable_event_queue` / `next_event(timeout_ms)` on the broker. Their smoke tests receive
messages and observe broker events that way. Authentication uses the pull-style auth queue (`enable_auth_queue` / `next_auth_request` / `resolve_auth_request`).

## ² Haskell generator patch

Upstream's Haskell generator had three bugs that made every binding unusable: constructors never
lowered their arguments (generated code didn't compile), flat-error variants dropped their message
(decoder failed with "left N trailing bytes"), and records sharing a field name broke the public module.
[`packaging/haskell/uniffi-bindgen-haskell.patch`](../packaging/haskell/uniffi-bindgen-haskell.patch)
fixes all three (and is proposed upstream, see [`packaging/haskell`](../packaging/haskell)); `install_bindgens.nu` builds the generator from the pinned revision with the patch applied.

## ³ Lua

Lua needs uniffi-bindgen-lua 0.1.2 or later: 0.1.1 deadlocked when Lua waited in one module (a
client's `connect`) while Rust called back into the other (the broker's auth provider).


## Sources-only languages

C, C++, Zig and Lua ship as sources (`bindings-<language>.zip` on each release) that you compile into
your program and link to the libraries of the `mqtt-client-` / `mqtt-broker-<version>-<target>` archives.


## One generator per language

 Each language has exactly one generator
(no parallel hand-written addons), and each component is self-contained: the broker owns its own `QoS`
enum instead of importing the client's, so no generator ever meets a cross-crate "external type" (the
Node generator rejects them) and every language gets a standalone broker package.


## UniFFI versions

UniFFI bindgens can only read metadata from the UniFFI release they were built
for, and the third-party generators disagree (the workspace and Java are on 0.32,
Haskell on 0.32.0, Go, C#, Dart and Node still on 0.31). `generate_bindings.nu` therefore builds the library for
such a generator in a scratch workspace under `target/uniffi-<version>/` pinned
to the right release; your workspace is never touched.

