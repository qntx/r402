//! Advertise-only `bazaar` discovery metadata.
//!
//! Resource servers declare how a paid HTTP endpoint or MCP tool is called.
//! This crate does not implement a facilitator `GET /discovery` client.

use compact_str::CompactString;
use r402_protocol::extension::{AdvertiseContext, Extension};
use r402_protocol::payment::ExtensionEntry;
use serde_json::{Value, json};

/// Stable extension key on the wire.
pub const BAZAAR_KEY: &str = "bazaar";

/// Request body encoding for HTTP `POST`/`PUT`/`PATCH` discovery info.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BazaarBodyType {
    /// `application/json` body.
    Json,
    /// Multipart form body.
    FormData,
    /// Plain-text body.
    Text,
}

impl BazaarBodyType {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::FormData => "form-data",
            Self::Text => "text",
        }
    }
}

/// Server-side `bazaar` declaration attached to `PaymentRequired.extensions`.
#[derive(Debug, Clone)]
pub struct BazaarExtension {
    info: Value,
    schema: Value,
}

impl BazaarExtension {
    /// HTTP query-parameter discovery (`GET`/`HEAD`/`DELETE`).
    #[must_use]
    pub fn http_query(query_params: Value) -> Self {
        Self {
            info: wrap_input(http_input([
                ("type", Value::from("http")),
                ("queryParams", query_params),
            ])),
            schema: query_schema(),
        }
    }

    /// HTTP body discovery (`POST`/`PUT`/`PATCH`).
    #[must_use]
    pub fn http_body(body_type: BazaarBodyType, body: Value) -> Self {
        Self {
            info: wrap_input(http_input([
                ("type", Value::from("http")),
                ("bodyType", Value::from(body_type.as_str())),
                ("body", body),
            ])),
            schema: body_schema(),
        }
    }

    /// MCP tool discovery.
    #[must_use]
    pub fn mcp(tool_name: impl Into<CompactString>, input_schema: Value) -> Self {
        let tool_name = tool_name.into();
        Self {
            info: wrap_input(http_input([
                ("type", Value::from("mcp")),
                ("toolName", Value::from(tool_name.as_str())),
                ("inputSchema", input_schema),
            ])),
            schema: mcp_schema(),
        }
    }

    /// Attaches an example JSON response under `info.output`.
    #[must_use]
    pub fn with_output(mut self, example: Value) -> Self {
        if let Some(obj) = self.info.as_object_mut() {
            let _ = obj.insert(
                "output".into(),
                http_input([("type", Value::from("json")), ("example", example)]),
            );
        }
        self
    }
}

fn http_input<const N: usize>(fields: [(&str, Value); N]) -> Value {
    Value::Object(fields.into_iter().map(|(k, v)| (k.to_owned(), v)).collect())
}

fn wrap_input(input: Value) -> Value {
    Value::Object(serde_json::Map::from_iter([("input".into(), input)]))
}

impl Extension for BazaarExtension {
    fn id(&self) -> &'static str {
        BAZAAR_KEY
    }

    fn advertise(&self, _ctx: &AdvertiseContext<'_>) -> Option<ExtensionEntry> {
        Some(ExtensionEntry::with_schema(
            self.info.clone(),
            self.schema.clone(),
        ))
    }
}

fn query_schema() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "input": {
                "type": "object",
                "properties": {
                    "type": { "type": "string", "const": "http" },
                    "method": { "type": "string", "enum": ["GET", "HEAD", "DELETE"] },
                    "queryParams": { "type": "object" },
                    "headers": { "type": "object", "additionalProperties": { "type": "string" } }
                },
                "required": ["type", "method"],
                "additionalProperties": false
            },
            "output": {
                "type": "object",
                "properties": {
                    "type": { "type": "string" },
                    "example": { "type": "object" }
                },
                "required": ["type"]
            }
        },
        "required": ["input"]
    })
}

fn body_schema() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "input": {
                "type": "object",
                "properties": {
                    "type": { "type": "string", "const": "http" },
                    "method": { "type": "string", "enum": ["POST", "PUT", "PATCH"] },
                    "bodyType": { "type": "string", "enum": ["json", "form-data", "text"] },
                    "body": { "type": "object" },
                    "queryParams": { "type": "object", "additionalProperties": { "type": "string" } },
                    "headers": { "type": "object", "additionalProperties": { "type": "string" } }
                },
                "required": ["type", "method", "bodyType", "body"],
                "additionalProperties": false
            },
            "output": {
                "type": "object",
                "properties": {
                    "type": { "type": "string" },
                    "example": { "type": "object" }
                },
                "required": ["type"]
            }
        },
        "required": ["input"]
    })
}

fn mcp_schema() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "input": {
                "type": "object",
                "properties": {
                    "type": { "type": "string", "const": "mcp" },
                    "toolName": { "type": "string" },
                    "description": { "type": "string" },
                    "transport": { "type": "string", "enum": ["streamable-http", "sse"] },
                    "inputSchema": { "type": "object" },
                    "example": { "type": "object" }
                },
                "required": ["type", "toolName", "inputSchema"],
                "additionalProperties": false
            },
            "output": {
                "type": "object",
                "properties": {
                    "type": { "type": "string" },
                    "example": { "type": "object" }
                },
                "required": ["type"]
            }
        },
        "required": ["input"]
    })
}
