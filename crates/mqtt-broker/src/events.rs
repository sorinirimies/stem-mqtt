//! Connect/disconnect/publish event notifications to the foreign-side
//! [`MqttBrokerEventListener`], decoupled from session/connection logic so
//! callers don't need to know *how* events reach observers, only that they
//! do. Events can also be *pulled* ([`BrokerEvent`] + `MqttBroker::next_event`)
//! by runtimes that can't receive callbacks from Rust threads.

use std::sync::Mutex;

use mqtt_client::support::{guard_callback, EventQueue, LockExt};
use mqtt_client::QoS;

use crate::config::SharedEventListener;

/// A broker event, as returned by the pull-style `MqttBroker::next_event`
/// (the polling twin of [`MqttBrokerEventListener`](crate::config::MqttBrokerEventListener)).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum BrokerEvent {
    ClientConnected {
        client_id: String,
    },
    ClientDisconnected {
        client_id: String,
        reason: String,
    },
    MessagePublished {
        client_id: String,
        topic: String,
        qos: QoS,
    },
}

/// Defines `EventHub` notification methods. Every one has the identical
/// shape — snapshot the listener out of the lock (never call foreign code
/// while holding it), convert borrowed arguments to the owned values the
/// foreign callback takes (via `Into`), run the callback with panics
/// contained, and mirror the event into the pull queue — so each is declared
/// by name, arguments, callback and the [`BrokerEvent`] it produces.
macro_rules! hub_notifiers {
    ($( $(#[$meta:meta])* fn $name:ident => $callback:ident ( $($arg:ident : $ty:ty),* ) => $event:ident;)+) => {
        $(
            $(#[$meta])*
            pub fn $name(&self $(, $arg: $ty)*) {
                self.queue.push(BrokerEvent::$event { $( $arg: $arg.into() ),* });
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
/// fans out lifecycle notifications to it (and to the optional pull queue).
///
/// A panic inside a foreign listener is contained and logged rather than
/// unwinding into the connection task that raised the event.
///
/// [`MqttBrokerEventListener`]: crate::config::MqttBrokerEventListener
#[derive(Default)]
pub struct EventHub {
    listener: Mutex<Option<SharedEventListener>>,
    pub queue: EventQueue<BrokerEvent>,
}

impl EventHub {
    pub fn new() -> Self {
        EventHub::default()
    }

    pub fn set_listener(&self, listener: SharedEventListener) {
        *self.listener.lock_safe() = Some(listener);
    }

    hub_notifiers! {
        fn notify_connected => on_client_connected(client_id: &str) => ClientConnected;
        fn notify_disconnected => on_client_disconnected(client_id: &str, reason: &str) => ClientDisconnected;
        fn notify_message_published => on_message_published(client_id: &str, topic: &str, qos: QoS) => MessagePublished;
    }
}
