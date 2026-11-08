//! Erlang/Elixir NIF for stem-mqtt, shipped as the `stem_mqtt` Hex package
//! (`packaging/elixir`).
//!
//! The BEAM must never block on Rust, so:
//! - async calls (`connect`, `publish`, `start`, …) return at once; the result
//!   arrives later as `{:stem_mqtt_reply, request_id, result}`;
//! - incoming messages and broker events are pushed to the owner process as
//!   `{:stem_mqtt, handle_id, event}` from Tokio threads;
//! - broker authentication goes through the broker's request queue, answered
//!   with `broker_resolve_auth`, never through a synchronous callback.
//!
//! Each client and broker monitors its owner: when the owner exits, the client
//! disconnects or the broker stops.

// rustler's `nif`/`resource_impl`/`init!` macros expand to `unsafe` FFI glue.
#![allow(unsafe_code)]

mod broker;
mod client;
mod options;

use std::future::Future;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

use rustler::{Encoder, Env, LocalPid, NifException, OwnedEnv, Term};
use tokio::runtime::{Builder, Runtime};

pub(crate) mod atoms {
    rustler::atoms! {
        ok,
        error,
        stem_mqtt,
        stem_mqtt_reply,
        message,
        disconnected,
        client_connected,
        client_disconnected,
        message_published,
        auth_request,
        v5,
        v311,
    }
}

/// The Tokio runtime every client and broker runs on, started on first use.
pub(crate) fn runtime() -> &'static Runtime {
    static RUNTIME: OnceLock<Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        Builder::new_multi_thread()
            .enable_all()
            .thread_name("stem-mqtt")
            .build()
            .expect("start the stem-mqtt Tokio runtime")
    })
}

/// A fresh id for a client or broker; events carry it so the owner can tell
/// several handles apart without the event keeping the handle alive.
pub(crate) fn next_handle_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// Raised for bad arguments, as an Elixir `ArgumentError`.
#[derive(NifException)]
#[module = "ArgumentError"]
pub(crate) struct ArgumentError {
    message: String,
}

pub(crate) fn argument_error(message: impl Into<String>) -> rustler::Error {
    rustler::Error::RaiseTerm(Box::new(ArgumentError {
        message: message.into(),
    }))
}

/// Sends `{:stem_mqtt, handle_id, event}` to `pid` from a non-BEAM thread.
/// A dead owner is not an error: the message is dropped, as `send/2` would.
pub(crate) fn notify<F>(pid: &LocalPid, handle_id: u64, event: F)
where
    F: for<'a> FnOnce(Env<'a>) -> Term<'a>,
{
    let mut env = OwnedEnv::new();
    // Ignored on purpose: the only failure is a dead recipient.
    let _ = env.send_and_clear(pid, |env| {
        (atoms::stem_mqtt(), handle_id, event(env)).encode(env)
    });
}

/// Runs `future` on the runtime and sends `{:stem_mqtt_reply, request, term}`
/// to the calling process when it completes.
pub(crate) fn spawn_reply<Fut, R, E>(env: Env, request: u64, future: Fut, encode: E)
where
    Fut: Future<Output = R> + Send + 'static,
    R: Send + 'static,
    E: for<'a> FnOnce(Env<'a>, R) -> Term<'a> + Send + 'static,
{
    let pid = env.pid();
    runtime().spawn(async move {
        let result = future.await;
        let mut env = OwnedEnv::new();
        // Ignored on purpose: the caller may have exited while waiting.
        let _ = env.send_and_clear(&pid, |env| {
            (atoms::stem_mqtt_reply(), request, encode(env, result)).encode(env)
        });
    });
}

/// `{:error, {kind, message}}`, the error shape of every async reply.
pub(crate) fn error_term<'a>(env: Env<'a>, kind: &str, message: String) -> Term<'a> {
    let kind = rustler::Atom::from_str(env, kind).expect("error kinds are valid atoms");
    (atoms::error(), (kind, message)).encode(env)
}

/// `bytes` as an Erlang binary (rustler would encode a `Vec<u8>` as a list).
pub(crate) fn binary<'a>(env: Env<'a>, bytes: &[u8]) -> Term<'a> {
    let mut out = rustler::NewBinary::new(env, bytes.len());
    out.as_mut_slice().copy_from_slice(bytes);
    rustler::Binary::from(out).encode(env)
}

rustler::init!("Elixir.StemMqtt.Native");
