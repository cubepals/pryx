// SPDX-License-Identifier: FSL-1.1-ALv2
//! Scoped native tool routes reuse the same service owners as MCP. Binary images stay PNG;
//! access responses and screenshots are explicitly non-cacheable.
use super::{browser, desktop, mounts, ssh, types::*};
use crate::{
    auth::Claims,
    context::{Agent, Failure},
};
use axum::{
    Json, Router,
    extract::{Extension, Path, State},
    http::{
        HeaderValue, StatusCode,
        header::{CACHE_CONTROL, CONTENT_TYPE},
    },
    response::{IntoResponse, Response},
    routing::{get, post},
};
use std::sync::Arc;

pub(crate) fn routes() -> Router<Arc<Agent>> {
    Router::new()
        .route("/_pryx/desktop", post(open_desktop))
        .route("/_pryx/desktop/screenshot", post(desktop_shot))
        .route("/_pryx/desktop/click", post(click))
        .route("/_pryx/desktop/move", post(move_to))
        .route("/_pryx/desktop/type", post(text))
        .route("/_pryx/desktop/key", post(key))
        .route("/_pryx/desktop/scroll", post(scroll))
        .route("/_pryx/browser", post(open_browser))
        .route("/_pryx/browser/screenshot", post(browser_shot))
        .route("/_pryx/ssh", post(open_ssh).delete(revoke_all))
        .route("/_pryx/ssh/{id}", get(view_ssh).delete(revoke_ssh))
        .route("/_pryx/ssh/{id}/expiry", post(expire_ssh))
        .route("/_pryx/ssh/{id}/connect", get(connect_ssh))
        .route("/_pryx/mounts", get(list_mounts).post(init_mounts))
        .layer(axum::middleware::map_response(|mut response: Response| async move {
            response.headers_mut().insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
            response
        }))
}
fn png(bytes: Vec<u8>) -> Response {
    ([(CONTENT_TYPE, "image/png")], bytes).into_response()
}
async fn open_desktop(
    State(agent): State<Arc<Agent>>,
    Extension(claims): Extension<Claims>,
) -> Result<Json<ToolAccess>, Failure> {
    Ok(Json(desktop::open(&agent, &claims).await?))
}
async fn desktop_shot(
    State(agent): State<Arc<Agent>>,
    Extension(claims): Extension<Claims>,
) -> Result<Response, Failure> {
    Ok(png(desktop::screenshot(&agent, &claims).await?))
}
async fn click(
    State(agent): State<Arc<Agent>>,
    Extension(claims): Extension<Claims>,
    Json(body): Json<Point>,
) -> Result<StatusCode, Failure> {
    desktop::click(&agent, &claims, body).await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn move_to(
    State(agent): State<Arc<Agent>>,
    Extension(claims): Extension<Claims>,
    Json(body): Json<Point>,
) -> Result<StatusCode, Failure> {
    desktop::move_to(&agent, &claims, body).await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn text(
    State(agent): State<Arc<Agent>>,
    Extension(claims): Extension<Claims>,
    Json(body): Json<Text>,
) -> Result<StatusCode, Failure> {
    desktop::text(&agent, &claims, body).await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn key(
    State(agent): State<Arc<Agent>>,
    Extension(claims): Extension<Claims>,
    Json(body): Json<Key>,
) -> Result<StatusCode, Failure> {
    desktop::key(&agent, &claims, body).await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn scroll(
    State(agent): State<Arc<Agent>>,
    Extension(claims): Extension<Claims>,
    Json(body): Json<Scroll>,
) -> Result<StatusCode, Failure> {
    desktop::scroll(&agent, &claims, body).await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn open_browser(
    State(agent): State<Arc<Agent>>,
    Extension(claims): Extension<Claims>,
) -> Result<Json<ToolAccess>, Failure> {
    Ok(Json(browser::open(&agent, &claims).await?))
}
async fn browser_shot(
    State(agent): State<Arc<Agent>>,
    Extension(claims): Extension<Claims>,
    Json(body): Json<BrowserShot>,
) -> Result<Response, Failure> {
    Ok(png(browser::screenshot(&agent, &claims, body).await?))
}
async fn open_ssh(
    State(agent): State<Arc<Agent>>,
    Extension(claims): Extension<Claims>,
) -> Result<Json<SshCreated>, Failure> {
    Ok(Json(ssh::create(&agent, &claims).await?))
}
async fn view_ssh(State(agent): State<Arc<Agent>>, Path(id): Path<String>) -> Result<Json<SshView>, Failure> {
    Ok(Json(ssh::view(&agent, &id).await?))
}
async fn connect_ssh(
    State(agent): State<Arc<Agent>>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<String>,
    upgrade: axum::extract::ws::WebSocketUpgrade,
) -> Result<Response, Failure> {
    super::ssh_socket::connect(agent, claims, id, upgrade).await
}
async fn expire_ssh(
    State(agent): State<Arc<Agent>>,
    Path(id): Path<String>,
    Json(body): Json<SshExpiry>,
) -> Result<Json<SshView>, Failure> {
    Ok(Json(ssh::expire(&agent, &id, body.expires_at).await?))
}
async fn revoke_ssh(State(agent): State<Arc<Agent>>, Path(id): Path<String>) -> Result<StatusCode, Failure> {
    ssh::revoke(&agent, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn revoke_all(State(agent): State<Arc<Agent>>) -> Result<StatusCode, Failure> {
    let state = super::state::get(&agent)?;
    let ids: Vec<_> = state.ssh.lock().await.keys().cloned().collect();
    for id in ids {
        ssh::revoke(&agent, &id).await?;
    }
    Ok(StatusCode::NO_CONTENT)
}
async fn list_mounts(State(agent): State<Arc<Agent>>) -> Result<Json<serde_json::Value>, Failure> {
    Ok(Json(serde_json::json!({"mounts":mounts::list(&agent).await?})))
}
async fn init_mounts(
    State(agent): State<Arc<Agent>>,
    Json(body): Json<MountsInit>,
) -> Result<Json<serde_json::Value>, Failure> {
    Ok(Json(serde_json::json!({"mounts":mounts::initialize(&agent,body.mounts).await?})))
}

#[cfg(feature = "mcp")]
pub(crate) async fn mcp(
    agent: &Agent,
    claims: &Claims,
    name: &str,
    args: &serde_json::Value,
) -> Result<serde_json::Value, Failure> {
    fn parse<T: serde::de::DeserializeOwned>(args: &serde_json::Value) -> Result<T, Failure> {
        serde_json::from_value(args.clone())
            .map_err(|_| super::state::failure("invalid_request", "Invalid computer-use arguments"))
    }
    let image = match name {
        "screenshot" => Some(if args.get("url").is_some() {
            browser::screenshot(agent, claims, parse(args)?).await?
        } else {
            desktop::screenshot(agent, claims).await?
        }),
        "click" => {
            desktop::click(agent, claims, parse(args)?).await?;
            None
        }
        "type" => {
            desktop::text(agent, claims, parse(args)?).await?;
            None
        }
        "key" => {
            desktop::key(agent, claims, parse(args)?).await?;
            None
        }
        _ => return Err(super::state::failure("unknown_tool", "Unknown native tool")),
    };
    use base64::Engine;
    Ok(if let Some(image) = image {
        serde_json::json!({"content":[{"type":"image","mimeType":"image/png","data":base64::engine::general_purpose::STANDARD.encode(image)}],"isError":false})
    } else {
        serde_json::json!({"content":[{"type":"text","text":"Completed"}],"isError":false})
    })
}
