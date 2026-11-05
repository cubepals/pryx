// SPDX-License-Identifier: FSL-1.1-ALv2
//! Optional, stateless MCP surface on the authenticated sandbox listener. It owns no host or
//! fleet client. JSON responses implement Streamable HTTP without a session or idle SSE feed.
//!
//! Parts (`mcp/`):
//! - `tools.rs`: sandbox-local tools using existing process, file and port owners.
mod tools;
use crate::{
    auth::Claims,
    context::{Agent, Failure},
};
use axum::{
    Json, Router,
    extract::{Extension, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
};
use serde_json::{Value, json};
use std::sync::Arc;

// The released reference SDK negotiates this initialization-based revision. Current clients
// can use its specified backward compatibility; no unsupported revision is advertised.
const PROTOCOL: &str = "2025-11-25";
const SUPPORTED: [&str; 3] = [PROTOCOL, "2025-06-18", "2025-03-26"];
pub(crate) fn routes() -> Router<Arc<Agent>> {
    Router::new().route("/_pryx/mcp", post(handle))
}
fn rpc_error(id: Value, code: i32, message: &str) -> Response {
    Json(json!({"jsonrpc":"2.0", "id":id, "error":{"code":code,"message":message}})).into_response()
}
async fn handle(
    State(agent): State<Arc<Agent>>,
    Extension(claims): Extension<Claims>,
    headers: HeaderMap,
    bytes: axum::body::Bytes,
) -> Response {
    if headers.get("mcp-protocol-version").and_then(|v| v.to_str().ok()).is_some_and(|v| !SUPPORTED.contains(&v)) {
        let mut response = rpc_error(Value::Null, -32000, "Unsupported protocol version");
        *response.status_mut() = StatusCode::BAD_REQUEST;
        return response;
    }
    if headers.contains_key(axum::http::header::ORIGIN) {
        return StatusCode::FORBIDDEN.into_response();
    }
    if !headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.split(';').next() == Some("application/json"))
    {
        return StatusCode::UNSUPPORTED_MEDIA_TYPE.into_response();
    }
    let value: Value = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(_) => return rpc_error(Value::Null, -32700, "Parse error"),
    };
    let id = value.get("id").cloned().unwrap_or(Value::Null);
    let Some(method) = value.get("method").and_then(Value::as_str) else {
        return rpc_error(id, -32600, "Invalid request");
    };
    if value["jsonrpc"] != "2.0" || !value.is_object() || !(id.is_null() || id.is_string() || id.is_number()) {
        return rpc_error(id, -32600, "Invalid request");
    }
    if value.get("id").is_none() {
        return if method.starts_with("notifications/") { StatusCode::ACCEPTED } else { StatusCode::BAD_REQUEST }
            .into_response();
    }
    let params = &value["params"];
    let result = match method {
        "initialize" => {
            json!({"protocolVersion":params["protocolVersion"].as_str().filter(|v| SUPPORTED.contains(v)).unwrap_or(PROTOCOL),"capabilities":{"tools":{},"resources":{}},"serverInfo":{"name":"pryx-agent","version":env!("CARGO_PKG_VERSION")}})
        }
        "ping" => json!({}),
        "tools/list" => tools::catalog(),
        "tools/call" => match tools::call(&agent, &claims, params).await {
            Ok(value) => json!({"content":[{"type":"text","text":value.to_string()}]}),
            Err(Failure(_, code, message)) => {
                json!({"isError":true,"content":[{"type":"text","text":json!({"code":code,"message":message}).to_string()}]})
            }
        },
        "resources/list" => json!({"resources":[
            {"uri":"pryx://sandbox/state","name":"Sandbox state","mimeType":"application/json"},
            {"uri":"pryx://sandbox/logs","name":"Entrypoint logs","mimeType":"application/json"}
        ]}),
        "resources/templates/list" => json!({"resourceTemplates":[]}),
        "resources/read" => {
            let uri = params["uri"].as_str().unwrap_or_default();
            let text = match uri {
                "pryx://sandbox/state" => {
                    json!({"id":agent.sandbox,"epoch":agent.epoch,"processes":agent.processes.list()})
                }
                "pryx://sandbox/logs" => json!(agent.logs.subscribe().0),
                _ => return rpc_error(id, -32602, "Unknown resource"),
            };
            json!({"contents":[{"uri":uri,"mimeType":"application/json","text":text.to_string()}]})
        }
        _ => return rpc_error(id, -32601, "Method not found"),
    };
    Json(json!({"jsonrpc":"2.0","id":id,"result":result})).into_response()
}
