// SPDX-License-Identifier: FSL-1.1-ALv2
//! Browser discovery exposes a token-protected CDP proxy, while screenshots use the official
//! Playwright client against private Chromium. Chromium's own sandbox is always required.
use super::{
    command, state,
    types::{BrowserShot, ToolAccess},
};
use crate::{
    auth::Claims,
    context::{Agent, Failure},
};
use serde_json::json;

async fn ready(agent: &Agent) -> Result<std::sync::Arc<state::State>, Failure> {
    state::native(agent, "Browser automation")?;
    let state = state::get(agent)?;
    state::require(&state, "browser")?;
    state::ready(agent, super::defaults::BROWSER_PORT).await?;
    Ok(state)
}
pub(crate) async fn open(agent: &Agent, claims: &Claims) -> Result<ToolAccess, Failure> {
    let state = ready(agent).await?;
    let _lock = state.access.lock().await;
    let token = state::nonce()?;
    let expires_at = state::expiry(claims)?;
    let url = agent.ports.expose(super::defaults::BROWSER_PORT, claims.max_ports).await?.url;
    state::write(&state, "browser-access.json", &json!({"token":token,"expiresAt":expires_at,"url":url}))?;
    Ok(ToolAccess { url: format!("{url}/?token={token}"), expires_at })
}
pub(crate) async fn screenshot(agent: &Agent, claims: &Claims, body: BrowserShot) -> Result<Vec<u8>, Failure> {
    let state = ready(agent).await?;
    let url = reqwest::Url::parse(&body.url)
        .map_err(|_| state::failure("invalid_request", "Browser URL must use HTTP or HTTPS"))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(state::failure("invalid_request", "Browser URL must use HTTP or HTTPS without credentials"));
    }
    let path = state.directory.path().join(format!("browser-{}.png", uuid::Uuid::new_v4()));
    let result = command::run(
        agent,
        claims,
        vec!["node".into(), "/opt/pryx/browser/screenshot.cjs".into(), body.url, path.to_string_lossy().into_owned()],
        None,
    )
    .await;
    if let Err(error) = result {
        let _ = tokio::fs::remove_file(&path).await;
        return Err(error);
    }
    command::image(&path).await
}
