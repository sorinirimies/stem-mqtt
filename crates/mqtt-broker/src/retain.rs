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

#[cfg(test)]
mod tests {
    use super::*;
    use mqtt_client::protocol::properties::Properties;

    fn publish(topic: &str, payload: &'static [u8]) -> PublishPacket {
        PublishPacket {
            dup: false,
            qos: QoS::AtLeastOnce,
            retain: true,
            topic: topic.into(),
            packet_id: Some(1),
            payload: Bytes::from_static(payload),
            properties: Properties::new(),
        }
    }

    fn topics(store: &RetainStore, filter: &str) -> Vec<String> {
        let mut t: Vec<String> = store.matching(filter).into_iter().map(|(t, _)| t).collect();
        t.sort();
        t
    }

    #[test]
    fn stores_the_latest_message_per_topic() {
        let store = RetainStore::new(0);
        store.update(&publish("a/b", b"one"));
        store.update(&publish("a/b", b"two"));
        let found = store.matching("a/b");
        assert_eq!(found.len(), 1);
        assert_eq!(&found[0].1.payload[..], b"two");
    }

    #[test]
    fn empty_payload_clears_the_topic() {
        let store = RetainStore::new(0);
        store.update(&publish("a/b", b"x"));
        store.update(&publish("a/b", b""));
        assert!(store.matching("#").is_empty());
    }

    #[test]
    fn matching_honours_wildcards() {
        let store = RetainStore::new(0);
        for t in ["a/b", "a/c", "x/y"] {
            store.update(&publish(t, b"v"));
        }
        assert_eq!(topics(&store, "a/+"), ["a/b", "a/c"]);
        assert_eq!(topics(&store, "#"), ["a/b", "a/c", "x/y"]);
        assert_eq!(topics(&store, "a/b"), ["a/b"]);
    }

    #[test]
    fn at_capacity_new_topics_are_dropped_but_existing_ones_update() {
        let store = RetainStore::new(2);
        store.update(&publish("one", b"1"));
        store.update(&publish("two", b"2"));
        store.update(&publish("three", b"3")); // over the cap: silently dropped
        assert_eq!(topics(&store, "#"), ["one", "two"]);
        store.update(&publish("one", b"updated")); // existing topic still updates
        assert_eq!(&store.matching("one")[0].1.payload[..], b"updated");
        store.update(&publish("two", b"")); // clearing frees a slot
        store.update(&publish("three", b"3"));
        assert_eq!(topics(&store, "#"), ["one", "three"]);
    }
}
