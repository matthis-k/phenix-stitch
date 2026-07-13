use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::result::{ErrorKind, ToolFailure};

#[allow(clippy::result_large_err)]
/// Deserialize MCP tool input `Value` into a typed struct.
///
/// All missing fields must have `#[serde(default)]` for ergonomic optional inputs.
/// Returns `ToolFailure` with `ErrorKind::InvalidInput` on deserialization error.
pub fn parse_tool_input<T: DeserializeOwned>(
    input: &Value,
    audit_id: &str,
) -> Result<T, ToolFailure> {
    serde_json::from_value(input.clone()).map_err(|e| {
        ToolFailure::new(
            ErrorKind::InvalidInput,
            format!("Invalid input: {e}"),
            audit_id,
        )
    })
}
