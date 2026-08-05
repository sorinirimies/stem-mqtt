//! Standalone `uniffi-bindgen` CLI for this crate: generates Kotlin/Swift/
//! Python bindings from the compiled library.
//!
//! Usage (after `cargo build --release`):
//!
//! ```sh
//! cargo run -p stem-mqtt-client --features uniffi/cli --bin uniffi-bindgen -- \
//!     generate --library target/release/libmqtt_client.so \
//!     --language kotlin --out-dir bindings/kotlin
//! ```

fn main() {
    uniffi::uniffi_bindgen_main()
}
