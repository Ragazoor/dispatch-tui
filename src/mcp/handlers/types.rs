use serde::{Deserialize, Serialize};
use serde_json::Value;

// ---------------------------------------------------------------------------
// JSON-RPC request / response types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Option<Value>,
    pub method: String,
    pub params: Option<Value>,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
}

impl JsonRpcResponse {
    pub(super) fn ok(id: Option<Value>, result: Value) -> Self {
        JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }

    pub(super) fn err(id: Option<Value>, code: i32, message: impl Into<String>) -> Self {
        JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// Flexible i64 deserializer (accepts both 4 and "4")
// ---------------------------------------------------------------------------

/// Claude Code sometimes sends integer MCP arguments as strings.
/// Shared visitor that accepts both native integers and string-encoded integers.
pub(super) struct FlexibleI64Visitor;

impl<'de> serde::de::Visitor<'de> for FlexibleI64Visitor {
    type Value = i64;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("an integer or a string containing an integer")
    }

    fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<i64, E> {
        Ok(v)
    }

    fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<i64, E> {
        i64::try_from(v).map_err(|_| E::custom(format!("u64 out of i64 range: {v}")))
    }

    fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<i64, E> {
        v.parse::<i64>()
            .map_err(|_| E::custom(format!("invalid integer string: {v}")))
    }
}

pub(super) fn deserialize_flexible_i64<'de, D>(deserializer: D) -> Result<i64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    deserializer.deserialize_any(FlexibleI64Visitor)
}

pub(super) fn deserialize_optional_flexible_i64<'de, D>(
    deserializer: D,
) -> Result<Option<i64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de;

    struct OptFlexI64;
    impl<'de> de::Visitor<'de> for OptFlexI64 {
        type Value = Option<i64>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("null, an integer, or a string integer")
        }
        fn visit_none<E: de::Error>(self) -> Result<Option<i64>, E> {
            Ok(None)
        }
        fn visit_unit<E: de::Error>(self) -> Result<Option<i64>, E> {
            Ok(None)
        }
        fn visit_some<D2: serde::Deserializer<'de>>(self, d: D2) -> Result<Option<i64>, D2::Error> {
            d.deserialize_any(FlexibleI64Visitor).map(Some)
        }
    }
    deserializer.deserialize_option(OptFlexI64)
}

/// Nullable string deserializer for `Option<Option<String>>` fields.
/// Used with `#[serde(default)]` to distinguish absent (→ outer None),
/// JSON null (→ Some(None) = clear), and a value (→ Some(Some(v)) = set).
pub(super) fn deserialize_nullable_string<'de, D>(
    deserializer: D,
) -> Result<Option<Option<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de;

    struct NullableString;
    impl<'de> de::Visitor<'de> for NullableString {
        type Value = Option<Option<String>>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("null or a string")
        }
        fn visit_none<E: de::Error>(self) -> Result<Option<Option<String>>, E> {
            Ok(Some(None))
        }
        fn visit_unit<E: de::Error>(self) -> Result<Option<Option<String>>, E> {
            Ok(Some(None))
        }
        fn visit_some<D2: serde::Deserializer<'de>>(
            self,
            d: D2,
        ) -> Result<Option<Option<String>>, D2::Error> {
            String::deserialize(d).map(|s| Some(Some(s)))
        }
        fn visit_str<E: de::Error>(self, v: &str) -> Result<Option<Option<String>>, E> {
            Ok(Some(Some(v.to_owned())))
        }
        fn visit_string<E: de::Error>(self, v: String) -> Result<Option<Option<String>>, E> {
            Ok(Some(Some(v)))
        }
    }
    deserializer.deserialize_option(NullableString)
}

/// Nullable flexible i64 deserializer for `Option<Option<i64>>` fields.
/// Used with `#[serde(default)]` to distinguish absent (→ outer None),
/// JSON null (→ Some(None) = clear), and a value (→ Some(Some(v)) = set).
pub(super) fn deserialize_nullable_flexible_i64<'de, D>(
    deserializer: D,
) -> Result<Option<Option<i64>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de;

    struct NullableFlexI64;
    impl<'de> de::Visitor<'de> for NullableFlexI64 {
        type Value = Option<Option<i64>>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("null, an integer, or a string integer")
        }
        fn visit_none<E: de::Error>(self) -> Result<Option<Option<i64>>, E> {
            Ok(Some(None))
        }
        fn visit_unit<E: de::Error>(self) -> Result<Option<Option<i64>>, E> {
            Ok(Some(None))
        }
        fn visit_some<D2: serde::Deserializer<'de>>(
            self,
            d: D2,
        ) -> Result<Option<Option<i64>>, D2::Error> {
            d.deserialize_any(FlexibleI64Visitor).map(|v| Some(Some(v)))
        }
    }
    deserializer.deserialize_option(NullableFlexI64)
}

// ---------------------------------------------------------------------------
// Argument parsing helper
// ---------------------------------------------------------------------------

pub(super) fn parse_args<T: serde::de::DeserializeOwned>(
    id: &Option<Value>,
    args: Value,
) -> Result<T, JsonRpcResponse> {
    serde_json::from_value(args)
        .map_err(|e| JsonRpcResponse::err(id.clone(), -32602, format!("Invalid arguments: {e}")))
}

// ---------------------------------------------------------------------------
// Project ID resolution helper
// ---------------------------------------------------------------------------

/// Resolve an optional `project_id` argument: use it if provided, otherwise
/// fetch the default project from the DB.
pub(super) fn resolve_project_id(
    id: &Option<Value>,
    opt_project_id: Option<i64>,
    db: &dyn crate::db::ProjectCrud,
) -> Result<crate::models::ProjectId, JsonRpcResponse> {
    match opt_project_id {
        Some(pid) => Ok(pid),
        None => db.get_default_project().map(|p| p.id).map_err(|e| {
            JsonRpcResponse::err(
                id.clone(),
                -32603,
                format!("Failed to get default project: {e}"),
            )
        }),
    }
}

// ---------------------------------------------------------------------------
// ServiceError → JsonRpcResponse conversion
// ---------------------------------------------------------------------------

pub(super) fn service_err_to_response(
    id: Option<Value>,
    err: crate::service::ServiceError,
) -> JsonRpcResponse {
    use crate::service::ServiceError;
    match err {
        ServiceError::Validation(msg) => JsonRpcResponse::err(id, -32602, msg),
        ServiceError::NotFound(msg) => JsonRpcResponse::err(id, -32602, msg),
        ServiceError::Internal(msg) => JsonRpcResponse::err(id, -32603, msg),
    }
}
