//! Connect/disconnect/publish event notifications to the foreign-side
//! [`MqttBrokerEventListener`], decoupled from session/connection logic so
//! callers don't need to know *how* events reach observers, only that they
//! do.

use std::sync::Mutex;

use mqtt_client::support::{guard_callback, LockExt};
use mqtt_client::QoS;

use crate::config::SharedEventListener;

/// Defines `EventHub` notification methods. Every one has the identical
/// shape — snapshot the listener out of the lock (never call foreign code
/// while holding it), convert borrowed arguments to the owned values the
/// foreign callback takes (via `Into`), and run the callback with panics
/// contained — so each is declared by name, arguments and callback only.
macro_rules! hub_notifiers {
    ($( $(#[$meta:meta])* fn $name:ident => $callback:ident ( $($arg:ident : $ty:ty),* ); )+) => {
        $(
            $(#[$meta])*
            pub fn $name(&self $(, $arg: $ty)*) {
                let listener = self.listener.lock_safe().clone();
                if let Some(listener) = listener {
                    guard_callback(stringify!($callback), || {
                        // `&str -> String` for text arguments, identity for `Copy` ones.
                        listener.$callback($( $arg.into() ),*)
                    });
                }
            }
        )+
    };
}

/// Holds the single registered [`MqttBrokerEventListener`], if any, and
/// fans out lifecycle notifications to it.
///
/// A panic inside a foreign listener is contained and logged rather than
/// unwinding into the connection task that raised the event.
///
/// [`MqttBrokerEventListener`]: crate::config::MqttBrokerEventListener
#[derive(Default)]
pub struct EventHub {
    listener: Mutex<Option<SharedEventListener>>,
}

impl EventHub {
    pub fn new() -> Self {
        EventHub::default()
    }

    pub fn set_listener(&self, listener: SharedEventListener) {
        *self.listener.lock_safe() = Some(listener);
    }

    hub_notifiers! {
        fn notify_connected => on_client_connected(client_id: &str);
        fn notify_disconnected => on_client_disconnected(client_id: &str, reason: &str);
        fn notify_message_published => on_message_published(client_id: &str, topic: &str, qos: QoS);
    }
}
