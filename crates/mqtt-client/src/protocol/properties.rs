//! MQTT 5.0 Properties (MQTT-5.0 §2.2.2). Absent entirely in MQTT 3.1.1.

use bytes::{Buf, BufMut, Bytes, BytesMut};

use super::macros::mqtt_properties;
use super::varint::{
    decode_binary, decode_utf8_string, decode_varint, encode_binary, encode_utf8_string,
    encode_varint, varint_len,
};
use crate::error::{MqttError, MqttResult};

// The table below is the single source of truth for every MQTT 5.0
// property (MQTT-5.0 §2.2.2.2): each line names its wire identifier, its
// `Property` variant, and its wire encoding `kind` exactly once, and
// `mqtt_properties!` (see `super::macros`) expands that into the `Property`
// enum plus its `identifier`/`encode`/`encoded_len`/`decode` bodies.
// Previously these four were hand-written, and had to be kept in sync by
// hand across all ~27 property identifiers.
mqtt_properties! {
    1  => PayloadFormatIndicator: u8,
    2  => MessageExpiryInterval: u32,
    3  => ContentType: str,
    8  => ResponseTopic: str,
    9  => CorrelationData: bin,
    11 => SubscriptionIdentifier: varint32,
    17 => SessionExpiryInterval: u32,
    18 => AssignedClientIdentifier: str,
    19 => ServerKeepAlive: u16,
    21 => AuthenticationMethod: str,
    22 => AuthenticationData: bin,
    23 => RequestProblemInformation: u8,
    24 => WillDelayInterval: u32,
    25 => RequestResponseInformation: u8,
    26 => ResponseInformation: str,
    28 => ServerReference: str,
    31 => ReasonString: str,
    33 => ReceiveMaximum: u16,
    34 => TopicAliasMaximum: u16,
    35 => TopicAlias: u16,
    36 => MaximumQos: u8,
    37 => RetainAvailable: u8,
    38 => UserProperty: strpair,
    39 => MaximumPacketSize: u32,
    40 => WildcardSubscriptionAvailable: u8,
    41 => SubscriptionIdentifierAvailable: u8,
    42 => SharedSubscriptionAvailable: u8,
}

fn read_u8(buf: &mut Bytes) -> MqttResult<u8> {
    if !buf.has_remaining() {
        return Err(truncated("u8 property"));
    }
    Ok(buf.get_u8())
}

fn read_u16(buf: &mut Bytes) -> MqttResult<u16> {
    if buf.remaining() < 2 {
        return Err(truncated("u16 property"));
    }
    Ok(buf.get_u16())
}

fn read_u32(buf: &mut Bytes) -> MqttResult<u32> {
    if buf.remaining() < 4 {
        return Err(truncated("u32 property"));
    }
    Ok(buf.get_u32())
}

fn truncated(what: &str) -> MqttError {
    MqttError::MalformedPacket(format!("truncated {what}"))
}

/// An ordered list of MQTT 5.0 properties attached to a packet. Encoded on
/// the wire as `Property Length (varint)` followed by the properties
/// themselves. Always empty (and encoded as a single `0x00` length byte)
/// for MQTT 3.1.1 packets — callers simply never populate it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Properties(pub Vec<Property>);

impl Properties {
    pub fn new() -> Self {
        Properties(Vec::new())
    }

    pub fn push(&mut self, prop: Property) {
        self.0.push(prop);
    }

    pub fn find_u32(&self, id_of: impl Fn(&Property) -> Option<u32>) -> Option<u32> {
        self.0.iter().find_map(id_of)
    }

    fn body_len(&self) -> usize {
        self.0.iter().map(|p| p.encoded_len()).sum()
    }

    /// Encode `Property Length` + all properties into `out`.
    pub fn encode(&self, out: &mut BytesMut) -> MqttResult<()> {
        let len = self.body_len() as u32;
        encode_varint(len, out)?;
        for prop in &self.0 {
            prop.encode(out)?;
        }
        Ok(())
    }

    /// Total encoded size including the leading length prefix.
    pub fn encoded_len(&self) -> usize {
        let len = self.body_len();
        varint_len(len as u32) + len
    }

    /// Decode `Property Length` + properties from the front of `buf`.
    pub fn decode(buf: &mut Bytes) -> MqttResult<Self> {
        let len = decode_varint(buf)?.ok_or_else(|| truncated("property length"))? as usize;
        if buf.remaining() < len {
            return Err(truncated("properties"));
        }
        let mut body = buf.copy_to_bytes(len);
        let mut props = Vec::new();
        while body.has_remaining() {
            let id = decode_varint(&mut body)?.ok_or_else(|| truncated("property identifier"))?;
            props.push(Property::decode(id, &mut body)?);
        }
        Ok(Properties(props))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn properties_roundtrip() {
        let mut props = Properties::new();
        props.push(Property::PayloadFormatIndicator(1));
        props.push(Property::MessageExpiryInterval(3600));
        props.push(Property::ContentType("application/json".into()));
        props.push(Property::UserProperty(("k".into(), "v".into())));
        props.push(Property::SubscriptionIdentifier(200_000));

        let mut out = BytesMut::new();
        props.encode(&mut out).unwrap();
        assert_eq!(out.len(), props.encoded_len());

        let mut bytes = out.freeze();
        let decoded = Properties::decode(&mut bytes).unwrap();
        assert_eq!(decoded, props);
    }

    #[test]
    fn empty_properties_encode_as_zero() {
        let props = Properties::new();
        let mut out = BytesMut::new();
        props.encode(&mut out).unwrap();
        assert_eq!(&out[..], &[0x00]);
    }

    #[test]
    fn unknown_property_identifier_errors() {
        let mut buf = BytesMut::new();
        encode_varint(1, &mut buf).unwrap(); // length = 1
        buf.put_u8(200); // unknown identifier
        let mut bytes = buf.freeze();
        assert!(Properties::decode(&mut bytes).is_err());
    }

    #[test]
    fn every_property_kind_roundtrips() {
        // Exercises every wire `kind` the `mqtt_properties!` table uses, not
        // just the handful covered by `properties_roundtrip` above.
        let samples = vec![
            Property::PayloadFormatIndicator(1),        // u8
            Property::ServerKeepAlive(30),              // u16
            Property::MessageExpiryInterval(600),       // u32
            Property::SubscriptionIdentifier(16_384),   // varint32 (2-byte boundary)
            Property::ContentType("text/plain".into()), // str
            Property::CorrelationData(Bytes::from_static(b"\x00\x01\x02")), // bin
            Property::UserProperty(("key".into(), "value".into())), // strpair
        ];

        for prop in samples {
            let mut out = BytesMut::new();
            prop.encode(&mut out).unwrap();
            assert_eq!(
                out.len(),
                prop.encoded_len(),
                "encoded_len mismatch for {prop:?}"
            );

            let id = decode_varint(&mut out.clone().freeze()).unwrap().unwrap();
            assert_eq!(id, prop.identifier());

            // Skip the identifier varint, then decode the body.
            let mut bytes = out.freeze();
            decode_varint(&mut bytes).unwrap();
            let decoded = Property::decode(prop.identifier(), &mut bytes).unwrap();
            assert_eq!(decoded, prop);
        }
    }
}
