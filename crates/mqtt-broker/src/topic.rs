//! Topic matching/validation now lives in the protocol module so the
//! client and broker share one implementation; re-exported here so the
//! broker's internal paths stay short.

pub use mqtt_client::protocol::topic::{is_valid_filter, is_valid_topic_name, topic_matches};
