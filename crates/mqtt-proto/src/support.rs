//! `Debug` helpers that never print passwords or private keys (so a stray `{:?}` or
//! `tracing::debug!(?options)` cannot leak credentials into logs). Re-exported by
//! `stem-mqtt-client` under its `support` module.

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

impl<T: Secret + ?Sized> core::fmt::Debug for Redacted<'_, T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.0.describe())
    }
}

/// Implements [`Debug`](core::fmt::Debug) for a struct, printing the listed
/// `fields` normally and the `secret` fields as `<redacted>`.
///
/// ```
/// use mqtt_proto::redacted_debug;
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
