//! Custom deserializers that accept both JSON numbers and JSON strings
//! for unsigned integer fields.
//!
//! Some MCP clients serialize integer arguments as strings (e.g. `"5000"`
//! instead of `5000`). The standard serde `u64` deserializer rejects strings,
//! producing "invalid type: string, expected u64" errors. The wrappers in
//! this module coerce string representations of integers transparently so
//! the tool argument structs accept both forms.

use std::borrow::Cow;

use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Deserializer, Serialize};

fn uint_schema_object() -> Schema {
    let mut map = serde_json::Map::new();
    map.insert("type".into(), serde_json::json!("integer"));
    map.insert("minimum".into(), serde_json::json!(0));
    Schema::from(map)
}

fn option_uint_schema_object() -> Schema {
    let mut null_obj = serde_json::Map::new();
    null_obj.insert("type".into(), serde_json::Value::String("null".into()));
    let mut int_obj = serde_json::Map::new();
    int_obj.insert("type".into(), serde_json::Value::String("integer".into()));
    int_obj.insert("minimum".into(), serde_json::Value::Number(0.into()));
    let alternatives = serde_json::Value::Array(vec![
        serde_json::Value::Object(null_obj),
        serde_json::Value::Object(int_obj),
    ]);
    let mut map = serde_json::Map::new();
    map.insert("anyOf".into(), alternatives);
    Schema::from(map)
}

/// Wrapper around `u64` that deserializes from either a JSON number or a
/// JSON string containing a decimal integer.
///
/// ```json
/// {"timeout_ms": 5000}       // number — works
/// {"timeout_ms": "5000"}    // string — also works
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FlexibleU64(pub u64);

impl JsonSchema for FlexibleU64 {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("FlexibleU64")
    }
    fn inline_schema() -> bool {
        true
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        uint_schema_object()
    }
}

impl From<FlexibleU64> for u64 {
    fn from(v: FlexibleU64) -> Self {
        v.0
    }
}

impl From<u64> for FlexibleU64 {
    fn from(v: u64) -> Self {
        FlexibleU64(v)
    }
}

impl Serialize for FlexibleU64 {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u64(self.0)
    }
}

impl<'de> Deserialize<'de> for FlexibleU64 {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = FlexibleU64;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("u64 or a string containing a decimal integer")
            }

            fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Self::Value, E> {
                Ok(FlexibleU64(v))
            }

            fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Self::Value, E> {
                u64::try_from(v)
                    .map(FlexibleU64)
                    .map_err(|_| E::custom(format!("negative value {v} not allowed for u64 field")))
            }

            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
                v.trim()
                    .parse::<u64>()
                    .map(FlexibleU64)
                    .map_err(|_| E::custom(format!("invalid u64 string: {v:?}")))
            }

            fn visit_string<E: serde::de::Error>(self, v: String) -> Result<Self::Value, E> {
                self.visit_str(&v)
            }
        }

        d.deserialize_any(Visitor)
    }
}

/// Wrapper around `Option<u64>` that deserializes from either a JSON number,
/// a JSON string containing a decimal integer, or JSON null.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FlexibleOptionU64(pub Option<u64>);

impl JsonSchema for FlexibleOptionU64 {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("FlexibleOptionU64")
    }
    fn inline_schema() -> bool {
        true
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        option_uint_schema_object()
    }
}

impl From<FlexibleOptionU64> for Option<u64> {
    fn from(v: FlexibleOptionU64) -> Self {
        v.0
    }
}

impl From<Option<u64>> for FlexibleOptionU64 {
    fn from(v: Option<u64>) -> Self {
        FlexibleOptionU64(v)
    }
}

impl Serialize for FlexibleOptionU64 {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self.0 {
            Some(v) => s.serialize_u64(v),
            None => s.serialize_none(),
        }
    }
}

impl<'de> Deserialize<'de> for FlexibleOptionU64 {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = FlexibleOptionU64;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("u64, null, or a string containing a decimal integer")
            }

            fn visit_none<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                Ok(FlexibleOptionU64(None))
            }

            fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                Ok(FlexibleOptionU64(None))
            }

            fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Self::Value, E> {
                Ok(FlexibleOptionU64(Some(v)))
            }

            fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Self::Value, E> {
                u64::try_from(v)
                    .map(|v| FlexibleOptionU64(Some(v)))
                    .map_err(|_| E::custom(format!("negative value {v} not allowed for u64 field")))
            }

            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
                let trimmed = v.trim();
                if trimmed.is_empty() {
                    return Ok(FlexibleOptionU64(None));
                }
                trimmed
                    .parse::<u64>()
                    .map(|v| FlexibleOptionU64(Some(v)))
                    .map_err(|_| E::custom(format!("invalid u64 string: {v:?}")))
            }

            fn visit_string<E: serde::de::Error>(self, v: String) -> Result<Self::Value, E> {
                self.visit_str(&v)
            }
        }

        d.deserialize_any(Visitor)
    }
}

/// Wrapper around `u32` that deserializes from either a JSON number or a
/// JSON string containing a decimal integer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FlexibleU32(pub u32);

impl JsonSchema for FlexibleU32 {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("FlexibleU32")
    }
    fn inline_schema() -> bool {
        true
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        uint_schema_object()
    }
}

impl From<FlexibleU32> for u32 {
    fn from(v: FlexibleU32) -> Self {
        v.0
    }
}

impl From<u32> for FlexibleU32 {
    fn from(v: u32) -> Self {
        FlexibleU32(v)
    }
}

impl Serialize for FlexibleU32 {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u32(self.0)
    }
}

impl<'de> Deserialize<'de> for FlexibleU32 {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = FlexibleU32;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("u32 or a string containing a decimal integer")
            }

            fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Self::Value, E> {
                u32::try_from(v)
                    .map(FlexibleU32)
                    .map_err(|_| E::custom(format!("value {v} out of range for u32")))
            }

            fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Self::Value, E> {
                u32::try_from(v)
                    .map(FlexibleU32)
                    .map_err(|_| E::custom(format!("value {v} out of range for u32")))
            }

            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
                v.trim()
                    .parse::<u32>()
                    .map(FlexibleU32)
                    .map_err(|_| E::custom(format!("invalid u32 string: {v:?}")))
            }

            fn visit_string<E: serde::de::Error>(self, v: String) -> Result<Self::Value, E> {
                self.visit_str(&v)
            }
        }

        d.deserialize_any(Visitor)
    }
}

/// Wrapper around `usize` that deserializes from either a JSON number or a
/// JSON string containing a decimal integer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FlexibleUsize(pub usize);

impl JsonSchema for FlexibleUsize {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("FlexibleUsize")
    }
    fn inline_schema() -> bool {
        true
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        uint_schema_object()
    }
}

impl From<FlexibleUsize> for usize {
    fn from(v: FlexibleUsize) -> Self {
        v.0
    }
}

impl From<usize> for FlexibleUsize {
    fn from(v: usize) -> Self {
        FlexibleUsize(v)
    }
}

impl Serialize for FlexibleUsize {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u64(self.0 as u64)
    }
}

impl<'de> Deserialize<'de> for FlexibleUsize {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = FlexibleUsize;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("usize or a string containing a decimal integer")
            }

            fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Self::Value, E> {
                usize::try_from(v)
                    .map(FlexibleUsize)
                    .map_err(|_| E::custom(format!("value {v} out of range for usize")))
            }

            fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Self::Value, E> {
                usize::try_from(v)
                    .map(FlexibleUsize)
                    .map_err(|_| E::custom(format!("value {v} out of range for usize")))
            }

            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
                v.trim()
                    .parse::<usize>()
                    .map(FlexibleUsize)
                    .map_err(|_| E::custom(format!("invalid usize string: {v:?}")))
            }

            fn visit_string<E: serde::de::Error>(self, v: String) -> Result<Self::Value, E> {
                self.visit_str(&v)
            }
        }

        d.deserialize_any(Visitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flexible_u64_from_number() {
        let v: FlexibleU64 = serde_json::from_str("5000").unwrap();
        assert_eq!(v.0, 5000);
    }

    #[test]
    fn flexible_u64_from_string() {
        let v: FlexibleU64 = serde_json::from_str(r#""5000""#).unwrap();
        assert_eq!(v.0, 5000);
    }

    #[test]
    fn flexible_u64_rejects_negative() {
        let err = serde_json::from_str::<FlexibleU64>("-1");
        assert!(err.is_err());
    }

    #[test]
    fn flexible_u64_rejects_garbage_string() {
        let err = serde_json::from_str::<FlexibleU64>(r#""abc""#);
        assert!(err.is_err());
    }

    #[test]
    fn flexible_u64_roundtrip_json() {
        let original = FlexibleU64(12345);
        let json = serde_json::to_string(&original).unwrap();
        assert_eq!(json, "12345");
        let roundtripped: FlexibleU64 = serde_json::from_str(&json).unwrap();
        assert_eq!(roundtripped, original);
    }

    #[test]
    fn flexible_option_u64_from_number() {
        let v: FlexibleOptionU64 = serde_json::from_str("5000").unwrap();
        assert_eq!(v.0, Some(5000));
    }

    #[test]
    fn flexible_option_u64_from_string() {
        let v: FlexibleOptionU64 = serde_json::from_str(r#""5000""#).unwrap();
        assert_eq!(v.0, Some(5000));
    }

    #[test]
    fn flexible_option_u64_from_null() {
        let v: FlexibleOptionU64 = serde_json::from_str("null").unwrap();
        assert_eq!(v.0, None);
    }

    #[test]
    fn flexible_option_u64_from_empty_string() {
        let v: FlexibleOptionU64 = serde_json::from_str(r#""""#).unwrap();
        assert_eq!(v.0, None);
    }

    #[test]
    fn flexible_option_u64_rejects_garbage_string() {
        let err = serde_json::from_str::<FlexibleOptionU64>(r#""xyz""#);
        assert!(err.is_err());
    }

    #[test]
    fn flexible_option_u64_roundtrip_some() {
        let original = FlexibleOptionU64(Some(42));
        let json = serde_json::to_string(&original).unwrap();
        assert_eq!(json, "42");
        let roundtripped: FlexibleOptionU64 = serde_json::from_str(&json).unwrap();
        assert_eq!(roundtripped, original);
    }

    #[test]
    fn flexible_option_u64_roundtrip_none() {
        let original = FlexibleOptionU64(None);
        let json = serde_json::to_string(&original).unwrap();
        assert_eq!(json, "null");
        let roundtripped: FlexibleOptionU64 = serde_json::from_str(&json).unwrap();
        assert_eq!(roundtripped, original);
    }

    #[test]
    fn flexible_u32_from_number() {
        let v: FlexibleU32 = serde_json::from_str("115200").unwrap();
        assert_eq!(v.0, 115200);
    }

    #[test]
    fn flexible_u32_from_string() {
        let v: FlexibleU32 = serde_json::from_str(r#""115200""#).unwrap();
        assert_eq!(v.0, 115200);
    }

    #[test]
    fn flexible_u32_rejects_overflow() {
        let err = serde_json::from_str::<FlexibleU32>(&u64::MAX.to_string());
        assert!(err.is_err());
    }

    #[test]
    fn flexible_usize_from_number() {
        let v: FlexibleUsize = serde_json::from_str("1024").unwrap();
        assert_eq!(v.0, 1024);
    }

    #[test]
    fn flexible_usize_from_string() {
        let v: FlexibleUsize = serde_json::from_str(r#""4096""#).unwrap();
        assert_eq!(v.0, 4096);
    }

    #[test]
    fn flexible_usize_rejects_negative() {
        let err = serde_json::from_str::<FlexibleUsize>("-1");
        assert!(err.is_err());
    }

    #[test]
    fn flexible_usize_rejects_garbage_string() {
        let err = serde_json::from_str::<FlexibleUsize>(r#""abc""#);
        assert!(err.is_err());
    }

    #[test]
    fn read_args_timeout_ms_accepts_stringified_number() {
        #[derive(serde::Deserialize)]
        #[allow(dead_code)]
        struct ReadArgsTest {
            connection_id: String,
            #[serde(default)]
            timeout_ms: FlexibleOptionU64,
            #[serde(default = "default_max")]
            max_bytes: FlexibleUsize,
        }
        fn default_max() -> FlexibleUsize {
            FlexibleUsize(1024)
        }

        let args: ReadArgsTest = serde_json::from_str(
            r#"{"connection_id":"abc","timeout_ms":"5000","max_bytes":"2048"}"#,
        )
        .unwrap();
        assert_eq!(args.timeout_ms.0, Some(5000));
        assert_eq!(args.max_bytes.0, 2048);
    }

    #[test]
    fn read_args_timeout_ms_accepts_plain_number() {
        #[derive(serde::Deserialize)]
        #[allow(dead_code)]
        struct ReadArgsTest {
            connection_id: String,
            #[serde(default)]
            timeout_ms: FlexibleOptionU64,
        }

        let args: ReadArgsTest =
            serde_json::from_str(r#"{"connection_id":"abc","timeout_ms":5000}"#).unwrap();
        assert_eq!(args.timeout_ms.0, Some(5000));
    }

    #[test]
    fn read_args_timeout_ms_accepts_null() {
        #[derive(serde::Deserialize)]
        #[allow(dead_code)]
        struct ReadArgsTest {
            connection_id: String,
            #[serde(default)]
            timeout_ms: FlexibleOptionU64,
        }

        let args: ReadArgsTest =
            serde_json::from_str(r#"{"connection_id":"abc","timeout_ms":null}"#).unwrap();
        assert_eq!(args.timeout_ms.0, None);
    }

    #[test]
    fn open_args_baud_rate_accepts_stringified_number() {
        #[derive(serde::Deserialize)]
        #[allow(dead_code)]
        struct OpenArgsTest {
            port: String,
            baud_rate: FlexibleU32,
        }

        let args: OpenArgsTest =
            serde_json::from_str(r#"{"port":"/dev/ttyUSB0","baud_rate":"115200"}"#).unwrap();
        assert_eq!(args.baud_rate.0, 115200);
    }
}
