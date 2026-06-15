//! NDJSON framing and JSON-RPC 2.0 message types.
//!
//! Each line on stdin is one JSON object.  Each line we emit to stdout is
//! one JSON object (response or notification).  No `Content-Length` headers
//! — Zed already speaks NDJSON over stdio for its existing extension
//! protocol, so we reuse that framing for the sidecar.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const PROTOCOL_VERSION: u32 = 1;

/// Methods recognised by the sidecar.
pub const METHOD_INITIALIZE: &str = "initialize";
pub const METHOD_UPDATE_FILE: &str = "update_file";
pub const METHOD_CLOSE_FILE: &str = "close_file";
pub const METHOD_LOOKUP: &str = "lookup";
pub const METHOD_CURSOR_CONTEXT: &str = "cursor_context";
pub const METHOD_DOC_LOOKUP: &str = "doc_lookup";
pub const METHOD_WORKSPACE_MACROS: &str = "workspace_macros";
pub const METHOD_PING: &str = "ping";

/// JSON-RPC error codes (subset of LSP / JSON-RPC standard).
pub mod error {
    pub const PARSE_ERROR: i32 = -32700;
    pub const INVALID_REQUEST: i32 = -32600;
    pub const METHOD_NOT_FOUND: i32 = -32601;
    pub const INVALID_PARAMS: i32 = -32602;
    pub const INTERNAL_ERROR: i32 = -32603;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub jsonrpc: String,
    pub id: Value,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

impl Request {
    pub fn is_notification(&self) -> bool {
        // JSON-RPC notifications have no `id`.  We still parse them through
        // the same struct (the id field is `Value` and may be `Null`).
        matches!(self.id, Value::Null)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseOk {
    pub jsonrpc: String,
    pub id: Value,
    pub result: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseErr {
    pub jsonrpc: String,
    pub id: Value,
    pub error: ErrorBody,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorBody {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl ResponseOk {
    pub fn new(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result,
        }
    }
}

impl ResponseErr {
    pub fn new(id: Value, code: i32, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            error: ErrorBody {
                code,
                message: message.into(),
                data: None,
            },
        }
    }
}

/// Try to parse one NDJSON line into a `Request`.  Returns `None` for
/// blank lines (so the loop can skip them).
pub fn parse_line(line: &str) -> Result<Option<Request>, serde_json::Error> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let r: Request = serde_json::from_str(trimmed)?;
    Ok(Some(r))
}
