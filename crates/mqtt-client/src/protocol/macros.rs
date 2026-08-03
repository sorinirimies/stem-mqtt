//! Declarative macros that collapse MQTT's repeated "id ↔ value" wire
//! tables into a single source of truth, so the numeric identifiers from
//! the spec are listed exactly once instead of once per function.
//!
//! Match arms cannot come from a macro invoked *inside* someone else's
//! `match { .. }` block (`macro!() => expr` isn't valid arm syntax, and
//! macros can't expand to patterns either) — so [`mqtt_properties`]'s
//! `encode`/`encoded_len`/`decode` bodies use the standard "internal rules"
//! token-munching pattern: a recursive `@step` rule consumes one table
//! entry at a time and appends a complete, self-contained `pattern =>
//! expr,` arm to a bracketed accumulator; the terminal rule then splices
//! the fully-accumulated arm list into one literal `match { .. }` written
//! directly in the macro's own expansion (see
//! <https://veykril.github.io/tlborm/decl-macros/patterns/push-down-accum.html>).

/// Defines a `#[repr(u8)]` enum together with a `from_u8` decoder generated
/// from the same id/variant list, so the two can never drift apart.
///
/// ```ignore
/// u8_enum! {
///     /// doc comment
///     pub enum PacketType, "packet type" {
///         Connect = 1,
///         ConnAck = 2,
///     }
/// }
/// ```
macro_rules! u8_enum {
    (
        $(#[$meta:meta])*
        $vis:vis enum $name:ident, $what:literal {
            $( $variant:ident = $val:literal ),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        #[repr(u8)]
        $vis enum $name {
            $( $variant = $val, )+
        }

        impl $name {
            #[allow(dead_code)]
            fn from_u8(v: u8) -> crate::error::MqttResult<Self> {
                Ok(match v {
                    $( $val => $name::$variant, )+
                    other => {
                        return Err(crate::error::MqttError::MalformedPacket(format!(
                            concat!("unknown ", $what, " {}"),
                            other
                        )))
                    }
                })
            }
        }
    };
}

pub(crate) use u8_enum;

/// Defines the MQTT 5.0 [`Property`](super::properties::Property) enum plus
/// its `identifier`/`encode`/`encoded_len`/`decode` bodies from a single
/// `id => Variant: kind` table (MQTT-5.0 §2.2.2.2), so adding, removing, or
/// reclassifying a property only ever touches one line instead of four
/// hand-written match statements that previously had to be kept in sync by
/// hand across ~27 property identifiers.
///
/// `kind` selects both the variant's field type(s) and its wire encoding:
///
/// | kind       | Rust type          | wire encoding                      |
/// |------------|--------------------|-------------------------------------|
/// | `u8`       | `u8`               | single byte                         |
/// | `u16`      | `u16`              | 2-byte big-endian                   |
/// | `u32`      | `u32`              | 4-byte big-endian                   |
/// | `varint32` | `u32`              | MQTT variable byte integer          |
/// | `str`      | `String`           | 2-byte length + UTF-8 bytes         |
/// | `bin`      | `Bytes`            | 2-byte length + raw bytes           |
/// | `strpair`  | `(String, String)` | two length-prefixed UTF-8 strings   |
macro_rules! mqtt_properties {
    ( $( $id:literal => $variant:ident : $kind:ident ),+ $(,)? ) => {
        /// A single MQTT 5.0 property. Only the identifiers defined by the
        /// spec are representable, which makes decoding of unknown
        /// identifiers a hard error (as required by MQTT-5.0 §2.2.2.2).
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub enum Property {
            $( $variant( mqtt_properties!(@ty $kind) ), )+
        }

        impl Property {
            // Every variant has exactly one discriminant regardless of its
            // field arity, so this arm list can be written directly (no
            // per-kind dispatch needed) — `(..)` matches any field count.
            fn identifier(&self) -> u32 {
                match self {
                    $( Property::$variant(..) => $id, )+
                }
            }

            fn encode(&self, out: &mut BytesMut) -> MqttResult<()> {
                encode_varint(self.identifier(), out)?;
                mqtt_properties!(@encode self, out, []; $( $variant : $kind )+)
            }

            fn encoded_len(&self) -> usize {
                let id_len = varint_len(self.identifier());
                let body_len = mqtt_properties!(@len self, []; $( $variant : $kind )+);
                id_len + body_len
            }

            fn decode(id: u32, buf: &mut Bytes) -> MqttResult<Self> {
                mqtt_properties!(@decode id, buf, []; $( $id => $variant : $kind )+)
            }
        }
    };

    // ── field type per kind ──────────────────────────────────────────────
    (@ty u8) => { u8 };
    (@ty u16) => { u16 };
    (@ty u32) => { u32 };
    (@ty varint32) => { u32 };
    (@ty str) => { String };
    (@ty bin) => { Bytes };
    (@ty strpair) => { (String, String) };

    // ── encode(): push-down accumulator over one match expression ───────
    // The accumulator is a bracketed `[$($acc:tt)*]` group (one delimited
    // token tree) rather than a bare trailing `$($acc:tt)*`, and each
    // per-kind rule's own `$($rest:tt)*` is the very last thing it matches
    // — both are required to avoid "ambiguous tt-repetition boundary"
    // errors from rustc's macro matcher.
    (@encode $self:ident, $out:ident, [$($acc:tt)*]; $variant:ident : u8 $($rest:tt)*) => {
        mqtt_properties!(@encode $self, $out, [$($acc)*
            Property::$variant(v) => { $out.put_u8(*v); Ok(()) },
        ]; $($rest)*)
    };
    (@encode $self:ident, $out:ident, [$($acc:tt)*]; $variant:ident : u16 $($rest:tt)*) => {
        mqtt_properties!(@encode $self, $out, [$($acc)*
            Property::$variant(v) => { $out.put_u16(*v); Ok(()) },
        ]; $($rest)*)
    };
    (@encode $self:ident, $out:ident, [$($acc:tt)*]; $variant:ident : u32 $($rest:tt)*) => {
        mqtt_properties!(@encode $self, $out, [$($acc)*
            Property::$variant(v) => { $out.put_u32(*v); Ok(()) },
        ]; $($rest)*)
    };
    (@encode $self:ident, $out:ident, [$($acc:tt)*]; $variant:ident : varint32 $($rest:tt)*) => {
        mqtt_properties!(@encode $self, $out, [$($acc)*
            Property::$variant(v) => encode_varint(*v, $out),
        ]; $($rest)*)
    };
    (@encode $self:ident, $out:ident, [$($acc:tt)*]; $variant:ident : str $($rest:tt)*) => {
        mqtt_properties!(@encode $self, $out, [$($acc)*
            Property::$variant(v) => encode_utf8_string(v, $out),
        ]; $($rest)*)
    };
    (@encode $self:ident, $out:ident, [$($acc:tt)*]; $variant:ident : bin $($rest:tt)*) => {
        mqtt_properties!(@encode $self, $out, [$($acc)*
            Property::$variant(v) => encode_binary(v, $out),
        ]; $($rest)*)
    };
    (@encode $self:ident, $out:ident, [$($acc:tt)*]; $variant:ident : strpair $($rest:tt)*) => {
        mqtt_properties!(@encode $self, $out, [$($acc)*
            Property::$variant((k, v)) => { encode_utf8_string(k, $out)?; encode_utf8_string(v, $out) },
        ]; $($rest)*)
    };
    (@encode $self:ident, $out:ident, [$($acc:tt)*];) => {
        match $self { $($acc)* }
    };

    // ── encoded_len(): push-down accumulator over one match expression ──
    (@len $self:ident, [$($acc:tt)*]; $variant:ident : u8 $($rest:tt)*) => {
        mqtt_properties!(@len $self, [$($acc)* Property::$variant(_) => 1,]; $($rest)*)
    };
    (@len $self:ident, [$($acc:tt)*]; $variant:ident : u16 $($rest:tt)*) => {
        mqtt_properties!(@len $self, [$($acc)* Property::$variant(_) => 2,]; $($rest)*)
    };
    (@len $self:ident, [$($acc:tt)*]; $variant:ident : u32 $($rest:tt)*) => {
        mqtt_properties!(@len $self, [$($acc)* Property::$variant(_) => 4,]; $($rest)*)
    };
    (@len $self:ident, [$($acc:tt)*]; $variant:ident : varint32 $($rest:tt)*) => {
        mqtt_properties!(@len $self, [$($acc)* Property::$variant(v) => varint_len(*v),]; $($rest)*)
    };
    (@len $self:ident, [$($acc:tt)*]; $variant:ident : str $($rest:tt)*) => {
        mqtt_properties!(@len $self, [$($acc)* Property::$variant(v) => 2 + v.len(),]; $($rest)*)
    };
    (@len $self:ident, [$($acc:tt)*]; $variant:ident : bin $($rest:tt)*) => {
        mqtt_properties!(@len $self, [$($acc)* Property::$variant(v) => 2 + v.len(),]; $($rest)*)
    };
    (@len $self:ident, [$($acc:tt)*]; $variant:ident : strpair $($rest:tt)*) => {
        mqtt_properties!(@len $self, [$($acc)* Property::$variant((k, v)) => 2 + k.len() + 2 + v.len(),]; $($rest)*)
    };
    (@len $self:ident, [$($acc:tt)*];) => {
        match $self { $($acc)* }
    };

    // ── decode(): push-down accumulator, returns MqttResult<Property> ────
    (@decode $id:ident, $buf:ident, [$($acc:tt)*]; $lit:literal => $variant:ident : u8 $($rest:tt)*) => {
        mqtt_properties!(@decode $id, $buf, [$($acc)* $lit => read_u8($buf).map(Property::$variant),]; $($rest)*)
    };
    (@decode $id:ident, $buf:ident, [$($acc:tt)*]; $lit:literal => $variant:ident : u16 $($rest:tt)*) => {
        mqtt_properties!(@decode $id, $buf, [$($acc)* $lit => read_u16($buf).map(Property::$variant),]; $($rest)*)
    };
    (@decode $id:ident, $buf:ident, [$($acc:tt)*]; $lit:literal => $variant:ident : u32 $($rest:tt)*) => {
        mqtt_properties!(@decode $id, $buf, [$($acc)* $lit => read_u32($buf).map(Property::$variant),]; $($rest)*)
    };
    (@decode $id:ident, $buf:ident, [$($acc:tt)*]; $lit:literal => $variant:ident : varint32 $($rest:tt)*) => {
        mqtt_properties!(@decode $id, $buf, [$($acc)*
            $lit => decode_varint($buf).and_then(|o| o.ok_or_else(|| truncated("varint property"))).map(Property::$variant),
        ]; $($rest)*)
    };
    (@decode $id:ident, $buf:ident, [$($acc:tt)*]; $lit:literal => $variant:ident : str $($rest:tt)*) => {
        mqtt_properties!(@decode $id, $buf, [$($acc)* $lit => decode_utf8_string($buf).map(Property::$variant),]; $($rest)*)
    };
    (@decode $id:ident, $buf:ident, [$($acc:tt)*]; $lit:literal => $variant:ident : bin $($rest:tt)*) => {
        mqtt_properties!(@decode $id, $buf, [$($acc)* $lit => decode_binary($buf).map(Property::$variant),]; $($rest)*)
    };
    (@decode $id:ident, $buf:ident, [$($acc:tt)*]; $lit:literal => $variant:ident : strpair $($rest:tt)*) => {
        mqtt_properties!(@decode $id, $buf, [$($acc)*
            $lit => decode_utf8_string($buf).and_then(|k| Ok(Property::$variant((k, decode_utf8_string($buf)?)))),
        ]; $($rest)*)
    };
    (@decode $id:ident, $buf:ident, [$($acc:tt)*];) => {
        match $id {
            $($acc)*
            other => Err(MqttError::MalformedPacket(format!("unknown property identifier {other}"))),
        }
    };
}

pub(crate) use mqtt_properties;
