//! Connect/disconnect/publish event notifications to the foreign-side
//! [`MqttBrokerEventListener`], decoupled from session/connection logic so
//! callers don't need to know *how* events reach observers, only that they
//! do.

use std::sync::Mutex;

use mqtt_client::QoS;

use crate::config::SharedEventListener;

/// Holds the single registered [`MqttBrokerEventListener`], if any, and
/// fans out lifecycle notifications to it.
#[derive(Default)]
pub struct EventHub {
    listener: Mutex<Option<SharedEventListener>>,
}

impl EventHub {
    pub fn new() -> Self {
        EventHub {
            listener: Mutex::new(None),
        }
    }

    pub fn set_listener(&self, listener: SharedEventListener) {
        *self.listener.lock().unwrap() = Some(listener);
    }

    pub fn notify_connected(&self, client_id: &str) {
        if let Some(listener) = self.listener.lock().unwrap().clone() {
            listener.on_client_connected(client_id.to_string());
        }
    }

    pub fn notify_disconnected(&self, client_id: &str, reason: &str) {
        if let Some(listener) = self.listener.lock().unwrap().clone() {
            listener.on_client_disconnected(client_id.to_string(), reason.to_string());
        }
    }

    pub fn notify_message_published(&self, client_id: &str, topic: &str, qos: QoS) {
        if let Some(listener) = self.listener.lock().unwrap().clone() {
            listener.on_message_published(client_id.to_string(), topic.to_string(), qos);
        }
    }
}
