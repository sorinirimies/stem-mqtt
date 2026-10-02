//! Small cross-crate support utilities shared by `mqtt-client`,
//! `mqtt-broker` and the language-binding crates.
//!
//! Everything here exists to give one audited implementation of a pattern
//! that would otherwise be copy-pasted at every call site:
//!
//! - [`redacted_debug!`](crate::redacted_debug) / [`Secret`] — `Debug` impls
//!   that never print passwords or private keys (so a stray `{:?}` or
//!   `tracing::debug!(?options)` can't leak credentials into logs).
//! - [`guard_callback`] — runs a foreign (Kotlin/Swift/Python/Go/…)
//!   callback without letting a panic unwind into an I/O task.
//! - [`LockExt`] / [`RwLockExt`] — poison-tolerant lock access. A panic in
//!   one connection task must not turn every later `lock().unwrap()` in the
//!   process into another panic.
//! - [`secs_or`] — the "`0` means use the built-in default" convention used
//!   by every `*_secs` configuration field.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::time::Duration;

use bytes::Bytes;

/// Describes a secret value for `Debug` output without revealing it.
///
/// Implemented for the shapes secrets take in this workspace
/// (`Option<_>`, `Vec<u8>`, `Bytes`, `String`). `Option` keeps its
/// "is it set at all?" signal, which is usually what you want when
/// debugging a configuration problem.
pub trait Secret {
    /// A fixed, non-revealing description of the value.
    fn describe(&self) -> &'static str;
}

impl<T> Secret for Option<T> {
    fn describe(&self) -> &'static str {
        if self.is_some() {
            "Some(<redacted>)"
        } else {
            "None"
        }
    }
}

macro_rules! always_redacted {
    ($($ty:ty),+ $(,)?) => {
        $( impl Secret for $ty {
            fn describe(&self) -> &'static str { "<redacted>" }
        } )+
    };
}
always_redacted!(Vec<u8>, Bytes, String);

/// `Debug` adapter used by [`redacted_debug!`](crate::redacted_debug).
pub struct Redacted<'a, T: Secret + ?Sized>(pub &'a T);

impl<T: Secret + ?Sized> std::fmt::Debug for Redacted<'_, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0.describe())
    }
}

/// Implements [`Debug`](std::fmt::Debug) for a struct, printing the listed
/// `fields` normally and the `secret` fields as `<redacted>`.
///
/// ```
/// use mqtt_client::redacted_debug;
///
/// struct Login { user: String, password: Option<Vec<u8>> }
/// redacted_debug!(Login { user } secret { password });
///
/// let l = Login { user: "bob".into(), password: Some(b"hunter2".to_vec()) };
/// let shown = format!("{l:?}");
/// assert!(shown.contains("bob"));
/// assert!(!shown.contains("hunter2"));
/// ```
#[macro_export]
macro_rules! redacted_debug {
    ($ty:ident { $($field:ident),* $(,)? } secret { $($secret:ident),+ $(,)? }) => {
        impl ::core::fmt::Debug for $ty {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                f.debug_struct(stringify!($ty))
                    $( .field(stringify!($field), &self.$field) )*
                    $( .field(stringify!($secret), &$crate::support::Redacted(&self.$secret)) )+
                    .finish()
            }
        }
    };
}

/// Run a foreign callback, converting a panic into a logged error.
///
/// UniFFI turns an exception thrown by foreign code inside a callback
/// interface into a Rust panic. Without this guard, that panic would
/// unwind through the connection's read loop and silently kill it,
/// leaving the connection looking alive while nothing is being read.
pub fn guard_callback<R>(what: &'static str, f: impl FnOnce() -> R) -> Option<R> {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(value) => Some(value),
        Err(_) => {
            tracing::error!(callback = what, "foreign callback panicked; ignoring");
            None
        }
    }
}

/// Poison-tolerant [`Mutex`] access: a poisoned lock hands back its data
/// instead of panicking, so one crashed task can't take the whole process
/// down with it.
pub trait LockExt<T> {
    fn lock_safe(&self) -> MutexGuard<'_, T>;
}

impl<T> LockExt<T> for Mutex<T> {
    fn lock_safe(&self) -> MutexGuard<'_, T> {
        self.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Poison-tolerant [`RwLock`] access; see [`LockExt`].
pub trait RwLockExt<T> {
    fn read_safe(&self) -> RwLockReadGuard<'_, T>;
    fn write_safe(&self) -> RwLockWriteGuard<'_, T>;
}

impl<T> RwLockExt<T> for RwLock<T> {
    fn read_safe(&self) -> RwLockReadGuard<'_, T> {
        self.read().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn write_safe(&self) -> RwLockWriteGuard<'_, T> {
        self.write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// `value` seconds as a [`Duration`], or `default_secs` when `value == 0`
/// (the convention every `*_secs` option in this workspace follows).
pub fn secs_or(value: u32, default_secs: u32) -> Duration {
    Duration::from_secs(u64::from(if value == 0 { default_secs } else { value }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secs_or_uses_default_for_zero() {
        assert_eq!(secs_or(0, 15), Duration::from_secs(15));
        assert_eq!(secs_or(3, 15), Duration::from_secs(3));
    }

    #[test]
    fn guard_callback_swallows_panics() {
        assert_eq!(guard_callback("ok", || 7), Some(7));
        assert_eq!(guard_callback("boom", || -> u8 { panic!("boom") }), None);
    }

    #[test]
    fn poisoned_mutex_is_still_usable() {
        let m = std::sync::Arc::new(Mutex::new(1));
        let m2 = m.clone();
        let _ = std::thread::spawn(move || {
            let _g = m2.lock().unwrap();
            panic!("poison it");
        })
        .join();
        assert!(m.is_poisoned());
        assert_eq!(*m.lock_safe(), 1);
    }

    #[test]
    fn option_secret_keeps_presence_signal() {
        assert_eq!(Some(vec![1u8]).describe(), "Some(<redacted>)");
        assert_eq!(None::<Vec<u8>>.describe(), "None");
    }
}
