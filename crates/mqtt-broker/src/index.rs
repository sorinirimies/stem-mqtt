//! Subscription index: a topic-level trie mapping a published topic to the
//! subscriptions that match it **without scanning every session**.
//!
//! Before this existed, every PUBLISH walked every session and ran
//! `topic_matches` against every one of its filters — O(sessions × filters)
//! per message. Here a lookup visits only the trie nodes along the topic's own
//! levels (plus the `+` branches), so cost scales with topic depth and the
//! number of *matching* subscriptions.

use std::collections::HashMap;

use mqtt_client::protocol::topic::{classify_filter, FilterKind};

/// One subscription as stored in the index.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct IndexEntry {
    pub client_id: String,
    /// The key under which the session stores the subscription — the full
    /// filter string, `$share/<group>/…` prefix included.
    pub key: String,
    /// `Some(group)` for a shared subscription.
    pub group: Option<String>,
    /// The topic filter proper (without any `$share/<group>/` prefix).
    pub filter: String,
}

impl IndexEntry {
    /// Build the entry for `client_id`'s subscription stored under `key`.
    /// Returns `None` for a malformed shared filter.
    pub fn from_key(client_id: &str, key: &str) -> Option<Self> {
        let (group, filter) = match classify_filter(key) {
            FilterKind::Plain => (None, key),
            FilterKind::Shared { group, filter } => (Some(group.to_string()), filter),
            FilterKind::InvalidShared => return None,
        };
        Some(IndexEntry {
            client_id: client_id.to_string(),
            key: key.to_string(),
            group,
            filter: filter.to_string(),
        })
    }
}

#[derive(Default)]
struct Node {
    children: HashMap<String, Node>,
    /// The `+` child.
    plus: Option<Box<Node>>,
    /// Subscriptions whose filter ends exactly at this node.
    exact: Vec<IndexEntry>,
    /// Subscriptions whose filter is `<path to here>/#`.
    multi: Vec<IndexEntry>,
}

impl Node {
    fn is_empty(&self) -> bool {
        self.children.is_empty()
            && self.plus.is_none()
            && self.exact.is_empty()
            && self.multi.is_empty()
    }
}

/// Trie of every live subscription.
#[derive(Default)]
pub struct SubscriptionIndex {
    root: Node,
}

impl SubscriptionIndex {
    pub fn insert(&mut self, entry: IndexEntry) {
        let filter = entry.filter.clone();
        let mut node = &mut self.root;
        for level in filter.split('/') {
            match level {
                "#" => {
                    node.multi.push(entry);
                    return;
                }
                "+" => node = node.plus.get_or_insert_with(Default::default),
                other => node = node.children.entry(other.to_string()).or_default(),
            }
        }
        node.exact.push(entry);
    }

    /// Remove `client_id`'s subscription stored under `key`, pruning nodes
    /// that become empty.
    pub fn remove(&mut self, client_id: &str, key: &str) {
        let Some(entry) = IndexEntry::from_key(client_id, key) else {
            return;
        };
        let levels: Vec<&str> = entry.filter.split('/').collect();
        Self::remove_at(&mut self.root, &levels, &entry);
    }

    fn remove_at(node: &mut Node, levels: &[&str], entry: &IndexEntry) {
        let same = |e: &IndexEntry| e.client_id == entry.client_id && e.key == entry.key;
        match levels.split_first() {
            None => node.exact.retain(|e| !same(e)),
            Some((&"#", _)) => node.multi.retain(|e| !same(e)),
            Some((&"+", rest)) => {
                if let Some(child) = node.plus.as_deref_mut() {
                    Self::remove_at(child, rest, entry);
                    if child.is_empty() {
                        node.plus = None;
                    }
                }
            }
            Some((level, rest)) => {
                if let Some(child) = node.children.get_mut(*level) {
                    Self::remove_at(child, rest, entry);
                    if child.is_empty() {
                        node.children.remove(*level);
                    }
                }
            }
        }
    }

    /// Every subscription whose filter matches the concrete `topic`
    /// (MQTT-4.7: `+`, `#`, and the rule that a `$`-prefixed topic is never
    /// matched by a filter starting with a wildcard).
    pub fn matching(&self, topic: &str) -> Vec<IndexEntry> {
        let levels: Vec<&str> = topic.split('/').collect();
        let dollar = topic.starts_with('$');
        let mut out = Vec::new();
        Self::collect(&self.root, &levels, 0, dollar, &mut out);
        out
    }

    fn collect(
        node: &Node,
        levels: &[&str],
        depth: usize,
        dollar: bool,
        out: &mut Vec<IndexEntry>,
    ) {
        let wildcard_allowed = !(dollar && depth == 0);
        if wildcard_allowed {
            out.extend(node.multi.iter().cloned());
        }
        let Some(level) = levels.get(depth) else {
            out.extend(node.exact.iter().cloned());
            return;
        };
        if let Some(child) = node.children.get(*level) {
            Self::collect(child, levels, depth + 1, dollar, out);
        }
        if wildcard_allowed {
            if let Some(child) = node.plus.as_deref() {
                Self::collect(child, levels, depth + 1, dollar, out);
            }
        }
    }

    #[cfg(test)]
    fn is_empty(&self) -> bool {
        self.root.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::topic::topic_matches;

    fn entry(client: &str, key: &str) -> IndexEntry {
        IndexEntry::from_key(client, key).unwrap()
    }

    fn ids(index: &SubscriptionIndex, topic: &str) -> Vec<String> {
        let mut v: Vec<String> = index
            .matching(topic)
            .into_iter()
            .map(|e| e.client_id)
            .collect();
        v.sort();
        v
    }

    #[test]
    fn matches_exact_plus_and_hash() {
        let mut idx = SubscriptionIndex::default();
        idx.insert(entry("exact", "a/b/c"));
        idx.insert(entry("plus", "a/+/c"));
        idx.insert(entry("hash", "a/#"));
        idx.insert(entry("all", "#"));
        assert_eq!(ids(&idx, "a/b/c"), ["all", "exact", "hash", "plus"]);
        assert_eq!(ids(&idx, "a"), ["all", "hash"], "a/# matches a itself");
        assert_eq!(ids(&idx, "a/x/c"), ["all", "hash", "plus"]);
        assert_eq!(ids(&idx, "z"), ["all"]);
    }

    #[test]
    fn dollar_topics_skip_leading_wildcards() {
        let mut idx = SubscriptionIndex::default();
        idx.insert(entry("hash", "#"));
        idx.insert(entry("plus", "+/broker"));
        idx.insert(entry("sys", "$SYS/#"));
        assert_eq!(ids(&idx, "$SYS/broker"), ["sys"]);
    }

    #[test]
    fn shared_entries_carry_group_and_inner_filter() {
        let e = entry("c", "$share/workers/jobs/+");
        assert_eq!(e.group.as_deref(), Some("workers"));
        assert_eq!(e.filter, "jobs/+");
        let mut idx = SubscriptionIndex::default();
        idx.insert(e);
        assert_eq!(ids(&idx, "jobs/1"), ["c"]);
        assert!(IndexEntry::from_key("c", "$share/g").is_none());
    }

    #[test]
    fn removal_prunes_the_trie() {
        let mut idx = SubscriptionIndex::default();
        for key in ["a/b/c", "a/+/c", "a/#", "#", "x/y"] {
            idx.insert(entry("c", key));
        }
        for key in ["a/b/c", "a/+/c", "a/#", "#", "x/y"] {
            idx.remove("c", key);
        }
        assert!(idx.is_empty(), "no empty nodes left behind");
        assert!(idx.matching("a/b/c").is_empty());
    }

    #[test]
    fn removal_only_touches_the_named_client() {
        let mut idx = SubscriptionIndex::default();
        idx.insert(entry("one", "t"));
        idx.insert(entry("two", "t"));
        idx.remove("one", "t");
        assert_eq!(ids(&idx, "t"), ["two"]);
    }

    /// The index must agree with the brute-force matcher it replaces, for
    /// arbitrary filters and topics (deterministic xorshift, no extra deps).
    #[test]
    fn agrees_with_brute_force_matching() {
        let mut state = 0x9E3779B97F4A7C15u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let words = ["a", "b", "c", "$sys", ""];
        let random_levels = |n: &mut dyn FnMut() -> u64, wild: bool| {
            let depth = 1 + (n() % 4) as usize;
            (0..depth)
                .map(|i| {
                    let r = n() % 8;
                    if wild && r == 0 {
                        "+".to_string()
                    } else if wild && r == 1 && i == depth - 1 {
                        "#".to_string()
                    } else {
                        words[(n() % words.len() as u64) as usize].to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join("/")
        };

        for round in 0..300 {
            let mut idx = SubscriptionIndex::default();
            let mut filters = Vec::new();
            for client in 0..12 {
                let f = random_levels(&mut next, true);
                if crate::topic::is_valid_filter(&f) {
                    idx.insert(entry(&format!("c{client}"), &f));
                    filters.push((format!("c{client}"), f));
                }
            }
            for _ in 0..40 {
                let topic = random_levels(&mut next, false);
                if !crate::topic::is_valid_topic_name(&topic) {
                    continue;
                }
                let mut expected: Vec<String> = filters
                    .iter()
                    .filter(|(_, f)| topic_matches(f, &topic))
                    .map(|(c, _)| c.clone())
                    .collect();
                expected.sort();
                assert_eq!(
                    ids(&idx, &topic),
                    expected,
                    "round {round}: topic {topic:?} vs filters {filters:?}"
                );
            }
        }
    }
}
