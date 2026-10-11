//! Fuzzes topic-filter validation and matching, checking `topic_matches`
//! against an independent level-by-level reference implementation.
#![no_main]

use libfuzzer_sys::fuzz_target;
use mqtt_client::protocol::topic::{
    classify_filter, is_valid_filter, is_valid_topic_name, topic_matches,
};

/// Straight transcription of MQTT-5.0 §4.7 (no shared-subscription handling).
fn reference(filter: &str, topic: &str) -> bool {
    let f: Vec<&str> = filter.split('/').collect();
    let t: Vec<&str> = topic.split('/').collect();
    if topic.starts_with('$') && matches!(f[0], "+" | "#") {
        return false;
    }
    for (i, level) in f.iter().enumerate() {
        match *level {
            "#" => return true,
            "+" if i < t.len() => {}
            l if i < t.len() && l == t[i] => {}
            _ => return false,
        }
    }
    f.len() == t.len()
}

fuzz_target!(|data: (&str, &str)| {
    let (filter, topic) = data;
    let _ = classify_filter(filter);
    if is_valid_filter(filter) && is_valid_topic_name(topic) {
        if !filter.starts_with("$share/") {
            assert_eq!(
                topic_matches(filter, topic),
                reference(filter, topic),
                "filter {filter:?} topic {topic:?}"
            );
        } else {
            let _ = topic_matches(filter, topic);
        }
    }
});
