//! Fuzzes topic-filter validation and matching.
#![no_main]

use libfuzzer_sys::fuzz_target;
use mqtt_client::protocol::topic::{classify_filter, is_valid_filter, is_valid_topic_name, topic_matches};

fuzz_target!(|data: (&str, &str)| {
    let (filter, topic) = data;
    let _ = classify_filter(filter);
    if is_valid_filter(filter) && is_valid_topic_name(topic) {
        let _ = topic_matches(filter, topic);
    }
});
