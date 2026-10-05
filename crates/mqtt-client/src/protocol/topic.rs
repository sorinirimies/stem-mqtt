//! MQTT topic name / topic filter matching and validation (MQTT-3.1.1 §4.7;
//! MQTT-5.0 §4.7), including the `+` (single-level) and `#` (multi-level)
//! wildcards and the `$`-prefixed "system topic" exclusion rule.
//!
//! Lives in the protocol module (rather than the broker) so the client can
//! reject an illegal topic *before* it hits the wire — a broker must close
//! the connection on a PUBLISH with a wildcard topic, so catching it locally
//! turns a dropped connection into a clear error.

/// Maximum length of a topic name / filter in bytes (a 2-byte length prefix
/// on the wire, MQTT-3.1.1 §1.5.3).
pub const MAX_TOPIC_LEN: usize = u16::MAX as usize;

/// Returns `true` if `topic` (a concrete topic name, never containing
/// wildcards) matches `filter` (which may contain `+` and `#`).
pub fn topic_matches(filter: &str, topic: &str) -> bool {
    // Topics starting with `$` are never matched by a filter starting with
    // a wildcard (`+` or `#`), even though `+`/`#` would otherwise match
    // any single/multiple levels.
    if topic.starts_with('$') && (filter.starts_with('+') || filter.starts_with('#')) {
        return false;
    }

    let mut topic_levels = topic.split('/');
    let mut filter_levels = filter.split('/');

    loop {
        match (filter_levels.next(), topic_levels.next()) {
            (Some("#"), _) => return true, // '#' must be the last filter level; matches everything remaining.
            (Some("+"), Some(_)) => continue,
            (Some("+"), None) => return false,
            (Some(f), Some(t)) => {
                if f != t {
                    return false;
                }
            }
            (Some(_), None) => return false,
            (None, Some(_)) => return false,
            (None, None) => return true,
        }
    }
}

/// Prefix of an MQTT 5.0 shared subscription filter (`$share/<group>/<filter>`).
pub const SHARED_PREFIX: &str = "$share/";

/// What a subscription filter string denotes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterKind<'a> {
    /// An ordinary filter.
    Plain,
    /// A well-formed `$share/<group>/<filter>`.
    Shared { group: &'a str, filter: &'a str },
    /// Starts with `$share/` but is malformed (empty group, wildcard in the
    /// group name, or no filter after it) — MQTT-5.0 §4.8.2.
    InvalidShared,
}

/// Classify `filter` as plain, shared, or a malformed shared subscription.
pub fn classify_filter(filter: &str) -> FilterKind<'_> {
    let Some(rest) = filter.strip_prefix(SHARED_PREFIX) else {
        return FilterKind::Plain;
    };
    match rest.split_once('/') {
        Some((group, inner))
            if !group.is_empty() && !group.contains(['+', '#']) && !inner.is_empty() =>
        {
            FilterKind::Shared {
                group,
                filter: inner,
            }
        }
        _ => FilterKind::InvalidShared,
    }
}

/// Validate that `filter` is a syntactically legal subscription filter:
/// `#` may only appear as the final, whole level; `+` may only appear as a
/// whole level (not `a+` or `+a`).
pub fn is_valid_filter(filter: &str) -> bool {
    if filter.is_empty() || filter.len() > MAX_TOPIC_LEN || filter.contains('\0') {
        return false;
    }
    let levels: Vec<&str> = filter.split('/').collect();
    for (i, level) in levels.iter().enumerate() {
        if level.contains('#') && *level != "#" {
            return false;
        }
        if *level == "#" && i != levels.len() - 1 {
            return false;
        }
        if level.contains('+') && *level != "+" {
            return false;
        }
    }
    true
}

/// Validate that `topic` is a legal topic name for PUBLISH: non-empty and
/// free of wildcard characters, NULs, and no longer than [`MAX_TOPIC_LEN`].
pub fn is_valid_topic_name(topic: &str) -> bool {
    !topic.is_empty() && topic.len() <= MAX_TOPIC_LEN && !topic.contains(['+', '#', '\0'])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_match() {
        assert!(topic_matches("a/b/c", "a/b/c"));
        assert!(!topic_matches("a/b/c", "a/b/d"));
    }

    #[test]
    fn single_level_wildcard() {
        assert!(topic_matches("a/+/c", "a/b/c"));
        assert!(topic_matches("a/+/c", "a/x/c"));
        assert!(!topic_matches("a/+/c", "a/b/x/c"));
        assert!(!topic_matches("a/+", "a"));
        assert!(topic_matches("+", "a"));
    }

    #[test]
    fn multi_level_wildcard() {
        assert!(topic_matches("a/#", "a"));
        assert!(topic_matches("a/#", "a/b"));
        assert!(topic_matches("a/#", "a/b/c"));
        assert!(topic_matches("#", "anything/at/all"));
        assert!(!topic_matches("a/b/#", "a/x"));
    }

    #[test]
    fn dollar_topics_excluded_from_leading_wildcards() {
        assert!(!topic_matches("#", "$SYS/broker/uptime"));
        assert!(!topic_matches("+/broker/uptime", "$SYS/broker/uptime"));
        assert!(topic_matches("$SYS/#", "$SYS/broker/uptime"));
    }

    #[test]
    fn filter_validation() {
        assert!(is_valid_filter("a/b/#"));
        assert!(is_valid_filter("+/b/+"));
        assert!(is_valid_filter("#"));
        assert!(!is_valid_filter("a/#/b"));
        assert!(!is_valid_filter("a#"));
        assert!(!is_valid_filter("a+"));
        assert!(!is_valid_filter(""));
    }

    #[test]
    fn filter_classification() {
        assert_eq!(classify_filter("a/b"), FilterKind::Plain);
        assert_eq!(
            classify_filter("$share/g/a/#"),
            FilterKind::Shared {
                group: "g",
                filter: "a/#"
            }
        );
        for bad in [
            "$share/",
            "$share/g",
            "$share//a",
            "$share/g+/a",
            "$share/g/",
        ] {
            assert_eq!(classify_filter(bad), FilterKind::InvalidShared, "{bad}");
        }
        // Not a prefix match: `$sharex/..` and mid-string `$share/` are plain.
        assert_eq!(classify_filter("$sharex/a"), FilterKind::Plain);
    }

    #[test]
    fn topic_name_validation() {
        assert!(is_valid_topic_name("a/b/c"));
        assert!(!is_valid_topic_name("a/+/c"));
        assert!(!is_valid_topic_name("a/#"));
        assert!(!is_valid_topic_name(""));
        assert!(!is_valid_topic_name("a\0b"));
        assert!(!is_valid_topic_name(&"x".repeat(MAX_TOPIC_LEN + 1)));
    }
}
