//! Public-path compatibility: the codec moved to `stem-mqtt-proto` in 0.9.0, and everything that
//! 0.8.0 exported under `mqtt_client::protocol`, `mqtt_client::error` and `mqtt_client::support`
//! must still import from the same path.
//!
//! (`cargo semver-checks` reports these as removed, because it cannot follow a `pub use` of another
//! crate's items; this test is the real check. Add a line when a path becomes public.)
#![allow(unused_imports)]

use mqtt_client::MqttError as old_path_1;
use mqtt_client::MqttVersion as old_path_2;
use mqtt_client::QoS as old_path_3;
use mqtt_client::error::MqttError as old_path_4;
use mqtt_client::protocol::AuthPacket as old_path_5;
use mqtt_client::protocol::ConnAckPacket as old_path_6;
use mqtt_client::protocol::ConnectPacket as old_path_7;
use mqtt_client::protocol::ConnectReasonCode as old_path_8;
use mqtt_client::protocol::DisconnectPacket as old_path_9;
use mqtt_client::protocol::MqttVersion as old_path_10;
use mqtt_client::protocol::Packet as old_path_11;
use mqtt_client::protocol::PacketType as old_path_12;
use mqtt_client::protocol::Properties as old_path_13;
use mqtt_client::protocol::Property as old_path_14;
use mqtt_client::protocol::PublishPacket as old_path_15;
use mqtt_client::protocol::QoS as old_path_16;
use mqtt_client::protocol::RetainHandling as old_path_17;
use mqtt_client::protocol::SimpleAck as old_path_18;
use mqtt_client::protocol::SubAckPacket as old_path_19;
use mqtt_client::protocol::SubAckReasonCode as old_path_20;
use mqtt_client::protocol::SubscribeFilter as old_path_21;
use mqtt_client::protocol::SubscribePacket as old_path_22;
use mqtt_client::protocol::UnsubAckPacket as old_path_23;
use mqtt_client::protocol::UnsubscribePacket as old_path_24;
use mqtt_client::protocol::Will as old_path_25;
use mqtt_client::protocol::ack as old_path_26;
use mqtt_client::protocol::ack::SimpleAck as old_path_27;
use mqtt_client::protocol::connect as old_path_28;
use mqtt_client::protocol::connect::ConnAckPacket as old_path_29;
use mqtt_client::protocol::connect::ConnectPacket as old_path_30;
use mqtt_client::protocol::connect::ConnectReasonCode as old_path_31;
use mqtt_client::protocol::connect::Will as old_path_32;
use mqtt_client::protocol::packet as old_path_33;
use mqtt_client::protocol::packet::AuthPacket as old_path_34;
use mqtt_client::protocol::packet::DisconnectPacket as old_path_35;
use mqtt_client::protocol::packet::Packet as old_path_36;
use mqtt_client::protocol::packet::PacketType as old_path_37;
use mqtt_client::protocol::properties as old_path_38;
use mqtt_client::protocol::properties::Properties as old_path_39;
use mqtt_client::protocol::properties::Property as old_path_40;
use mqtt_client::protocol::publish as old_path_41;
use mqtt_client::protocol::publish::PublishPacket as old_path_42;
use mqtt_client::protocol::subscribe as old_path_43;
use mqtt_client::protocol::subscribe::RetainHandling as old_path_44;
use mqtt_client::protocol::subscribe::SubAckPacket as old_path_45;
use mqtt_client::protocol::subscribe::SubAckReasonCode as old_path_46;
use mqtt_client::protocol::subscribe::SubscribeFilter as old_path_47;
use mqtt_client::protocol::subscribe::SubscribePacket as old_path_48;
use mqtt_client::protocol::subscribe::UnsubAckPacket as old_path_49;
use mqtt_client::protocol::subscribe::UnsubscribePacket as old_path_50;
use mqtt_client::protocol::topic as old_path_51;
use mqtt_client::protocol::topic::FilterKind as old_path_52;
use mqtt_client::protocol::topic::classify_filter as old_path_53;
use mqtt_client::protocol::topic::is_valid_filter as old_path_54;
use mqtt_client::protocol::topic::is_valid_topic_name as old_path_55;
use mqtt_client::protocol::topic::topic_matches as old_path_56;
use mqtt_client::protocol::varint as old_path_57;
use mqtt_client::protocol::varint::decode_binary as old_path_58;
use mqtt_client::protocol::varint::decode_utf8_string as old_path_59;
use mqtt_client::protocol::varint::decode_varint as old_path_60;
use mqtt_client::protocol::varint::encode_binary as old_path_61;
use mqtt_client::protocol::varint::encode_utf8_string as old_path_62;
use mqtt_client::protocol::varint::encode_varint as old_path_63;
use mqtt_client::protocol::varint::peek_varint as old_path_64;
use mqtt_client::protocol::varint::varint_len as old_path_65;
use mqtt_client::redacted_debug as old_path_66;
use mqtt_client::support::Redacted as old_path_67;
use mqtt_client::support::Secret as old_path_68;

#[test]
fn paths_exported_by_0_8_0_still_import() {}
