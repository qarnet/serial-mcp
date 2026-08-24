//! JSON Schema helpers that suppress non-standard `format` keywords generated
//! by schemars for unsigned integer types.
//!
//! # Before adding `uN` or `Option<uN>` fields
//!
//! schemars 1.x emits a non-standard `"format": "uintN"` keyword for each
//! unsigned integer type (`uint8`, `uint16`, `uint32`, `uint64`, `uint`).
//! None is part of the JSON Schema specification. Validators (jsonschema, AJV,
//! and others) log a warning like
//!
//! ```text
//! unknown format "uint16" ignored in schema at path "#/properties/vid"
//! ```
//!
//! and silently drop the constraint. This causes the
//! `logs/unknown format "uintN" ignored` warnings that have appeared in this
//! crate multiple times.
//!
//! Every `uN` / `Option<uN>` field on a struct that derives
//! [`schemars::JsonSchema`] must use
//! `#[schemars(schema_with = "crate::schema_helpers::uint_schema")]`
//! (or `option_uint_schema` for `Option<uN>`).
//!
//! An optional field with `#[serde(skip_serializing_if = "Option::is_none")]`
//! also needs `#[serde(default)]` when `schema_with` obscures its `Option` type
//! from schemars. schemars 1.2.2 does not see through `schema_with`, so without
//! `default` the field lands in the schema's `required` array while
//! serialization still omits it. The default evaluates to `None` (skipped), so
//! no `"default"` key is emitted into the schema. See
//! `src/serial/port_info.rs` (vid/pid/interface) and the
//! `port_info_optional_usb_fields_not_required` regression test.
//!
//! Required regression guards are `serial::schema`, which performs a per-type
//! scan over known public `JsonSchema` types, and
//! `tools::tests::tool_schemas_have_no_nonstandard_uint_formats`, which scans
//! tool `outputSchema` strings for `uint`/`uint8`/`uint16`/`uint32`/`uint64`.
//! Keep both tests. Contributors must extend the type list when adding a new
//! `JsonSchema`-deriving struct with unsigned integer fields.

use schemars::{json_schema, Schema, SchemaGenerator};

use crate::limits::{MAX_READ_BYTES, MAX_TIMEOUT_MS, MIN_READ_BYTES};

/// Schema for unsigned integer fields without the non-standard `format`
/// keyword.
/// Emits `{"type": "integer", "minimum": 0}`.
pub fn uint_schema(_gen: &mut SchemaGenerator) -> Schema {
    json_schema!({
        "type": "integer",
        "minimum": 0
    })
}

/// Schema for `Option<usize>`/`Option<u32>`/`Option<u64>` without the
/// non-standard `format` keyword.
/// Emits `{"anyOf": [{"type": "null"}, {"type": "integer", "minimum": 0}]}`.
pub fn option_uint_schema(_gen: &mut SchemaGenerator) -> Schema {
    json_schema!({
        "anyOf": [
            {"type": "null"},
            {"type": "integer", "minimum": 0}
        ]
    })
}

pub fn option_timeout_ms_schema(_gen: &mut SchemaGenerator) -> Schema {
    json_schema!({
        "anyOf": [
            {"type": "null"},
            {"type": "integer", "minimum": 0, "maximum": MAX_TIMEOUT_MS}
        ]
    })
}

/// Schema for `no_new_rx_timeout_ms`: rejects 0 (must be > 0) or null (disabled).
pub fn option_positive_timeout_ms_schema(_gen: &mut SchemaGenerator) -> Schema {
    json_schema!({
        "anyOf": [
            {"type": "null"},
            {"type": "integer", "minimum": 1, "maximum": MAX_TIMEOUT_MS}
        ]
    })
}

pub fn timeout_ms_schema(_gen: &mut SchemaGenerator) -> Schema {
    json_schema!({
        "type": "integer",
        "minimum": 0,
        "maximum": MAX_TIMEOUT_MS
    })
}

pub fn read_max_buffered_bytes_schema(_gen: &mut SchemaGenerator) -> Schema {
    json_schema!({
        "type": "integer",
        "minimum": MIN_READ_BYTES,
        "maximum": MAX_READ_BYTES
    })
}

/// Schema for `Vec<u8>` arrays with item bounds 0 through 255, without
/// schemars' non-standard `format: uint8`.
pub fn byte_array_schema(_gen: &mut SchemaGenerator) -> Schema {
    json_schema!({
        "type": "array",
        "items": {
            "type": "integer",
            "minimum": 0,
            "maximum": 255
        }
    })
}
