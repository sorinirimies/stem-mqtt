//! Retained-message store (MQTT-3.3.1.3 / MQTT-5.0 §3.3.1.3).
//!
//! Owns exactly one responsibility: remembering the last retained PUBLISH
//! per topic, bounded by [`MqttBrokerConfig::max_retained_messages`], and
//! answering "which retained messages match this subscribe filter?". It
//! knows nothing about sessions, connections, or delivery — that's
//! [`crate::registry::SessionRegistry`]'s job.

use std::collections::HashMap;
use std::sync::Mutex;

use bytes::Bytes;
use mqtt_client::protocol::publish::PublishPacket;
use mqtt_client::QoS;

use crate::topic::topic_matches;

/// One retained message stored per topic.
#[derive(Debug, Clone)]
pub struct RetainedMessage {
    pub payload: Bytes,
    pub qos: QoS,
}

/// Thread-safe store of retained messages, one per topic.
pub struct RetainStore {
    retained: Mutex<HashMap<String, RetainedMessage>>,
    max_retained_messages: u32,
}

impl RetainStore {
    pub fn new(max_retained_messages: u32) -> Self {
        RetainStore {
            retained: Mutex::new(HashMap::new()),
            max_retained_messages,
        }
    }

    /// Apply a retained PUBLISH: an empty payload clears the topic's
    /// retained message (MQTT-3.3.1-10); otherwise it's stored, unless
    /// we're at capacity and this would be a new topic (silently dropped).
    pub fn update(&self, publish: &PublishPacket) {
        let mut retained = self.retained.lock().unwrap();
        if publish.payload.is_empty() {
            retained.remove(&publish.topic);
            return;
        }
        if self.max_retained_messages > 0
            && retained.len() as u32 >= self.max_retained_messages
            && !retained.contains_key(&publish.topic)
        {
            return; // at capacity; silently drop new retained topics
        }
        retained.insert(
            publish.topic.clone(),
            RetainedMessage {
                payload: publish.payload.clone(),
                qos: publish.qos,
            },
        );
    }

    /// Every retained message whose topic matches `filter`, as owned
    /// clones (cheap: `Bytes` is refcounted).
    pub fn matching(&self, filter: &str) -> Vec<(String, RetainedMessage)> {
        let retained = self.retained.lock().unwrap();
        retained
            .iter()
            .filter(|(topic, _)| topic_matches(filter, topic))
            .map(|(t, m)| (t.clone(), m.clone()))
            .collect()
    }
}
