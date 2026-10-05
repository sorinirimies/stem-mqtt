//! Connect/disconnect/publish event notifications to the foreign-side
//! [`MqttBrokerEventListener`], decoupled from session/connection logic so
//! callers don't need to know *how* events reach observers, only that they
//! do. Events can also be *pulled* ([`BrokerEvent`] + `MqttBroker::next_event`)
//! by runtimes that can't receive callbacks from Rust threads.

use std::sync::Mutex;

use crate::config::{QoS, SharedEventListener};
use mqtt_client::support::{guard_callback, EventQueue, LockExt};
use mqtt_client::QoS as CoreQoS;

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
                // Build the event (allocating its strings) only if someone is polling.
                if self.queue.is_enabled() {
                    self.queue.push(BrokerEvent::$event { $( $arg: $arg.into() ),* });
                }
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
        fn notify_message_published => on_message_published(client_id: &str, topic: &str, qos: CoreQoS) => MessagePublished;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::MqttBrokerEventListener;
    use std::sync::Arc;
    use std::time::Duration;

    #[derive(Default)]
    struct Recorder(Mutex<Vec<String>>);
    impl MqttBrokerEventListener for Recorder {
        fn on_client_connected(&self, client_id: String) {
            self.0.lock_safe().push(format!("up:{client_id}"));
        }
        fn on_client_disconnected(&self, client_id: String, reason: String) {
            self.0
                .lock_safe()
                .push(format!("down:{client_id}:{reason}"));
        }
        fn on_message_published(&self, client_id: String, topic: String, _qos: QoS) {
            self.0.lock_safe().push(format!("pub:{client_id}:{topic}"));
        }
    }

    struct Panicker;
    impl MqttBrokerEventListener for Panicker {
        fn on_client_connected(&self, _: String) {
            panic!("listener blew up");
        }
        fn on_client_disconnected(&self, _: String, _: String) {}
        fn on_message_published(&self, _: String, _: String, _: QoS) {}
    }

    #[test]
    fn every_notifier_reaches_the_listener_with_owned_arguments() {
        let hub = EventHub::new();
        let rec = Arc::new(Recorder::default());
        hub.set_listener(rec.clone());
        hub.notify_connected("c1");
        hub.notify_message_published("c1", "t/x", CoreQoS::AtLeastOnce);
        hub.notify_disconnected("c1", "bye");
        assert_eq!(*rec.0.lock_safe(), ["up:c1", "pub:c1:t/x", "down:c1:bye"]);
    }

    #[test]
    fn a_panicking_listener_is_contained() {
        let hub = EventHub::new();
        hub.set_listener(Arc::new(Panicker));
        hub.notify_connected("c"); // must not unwind into the caller
    }

    #[tokio::test]
    async fn events_are_queued_only_when_the_queue_is_enabled() {
        let hub = EventHub::new();
        hub.notify_connected("ignored"); // disabled: nothing is built or kept
        hub.queue.enable(4);
        hub.notify_connected("kept");
        assert_eq!(
            hub.queue.next(Duration::from_millis(50)).await,
            Some(BrokerEvent::ClientConnected {
                client_id: "kept".into()
            })
        );
        assert_eq!(hub.queue.next(Duration::from_millis(20)).await, None);
    }
}
