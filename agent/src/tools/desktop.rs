// SPDX-License-Identifier: FSL-1.1-ALv2
//! Desktop control delegates to xdotool/scrot. noVNC uses websockify's token policy adapter;
//! only a capability holder can mint a short-lived browser URL, and raw VNC stays private.
use super::{
    command, state,
    types::{Key, Point, Scroll, Text, ToolAccess},
};
use crate::{
    auth::Claims,
    context::{Agent, Failure},
};
use serde_json::json;

pub(crate) async fn open(agent: &Agent, claims: &Claims) -> Result<ToolAccess, Failure> {
    state::native(agent, "Desktop automation")?;
    let state = state::get(agent)?;
    state::require(&state, "desktop")?;
    state::ready(agent, super::defaults::DESKTOP_PORT).await?;
    let _lock = state.access.lock().await;
    let token = state::nonce()?;
    let expires_at = state::expiry(claims)?;
    // TokenFile owns token-to-target parsing; the adapter additionally verifies wall-clock expiry.
    use std::io::Write;
    let directory = crate::paths::DataDir::open(state.directory.path())?;
    directory.write("tokens")?.write_all(format!("{token}: 127.0.0.1:{}\n", super::defaults::VNC_PORT).as_bytes())?;
    state::write(&state, "desktop-access.json", &json!({"token":token,"expiresAt":expires_at}))?;
    let url = agent.ports.expose(super::defaults::DESKTOP_PORT, claims.max_ports).await?.url;
    let base = reqwest::Url::parse(&url).map_err(|_| state::failure("invalid_url", "Invalid public desktop URL"))?;
    let path = format!("{}/websockify?token={token}", base.path().trim_end_matches('/'));
    let path = percent_encoding::utf8_percent_encode(path.trim_start_matches('/'), percent_encoding::NON_ALPHANUMERIC);
    Ok(ToolAccess {
        url: format!("{}/vnc.html#autoconnect=true&resize=remote&path={path}", url.trim_end_matches('/')),
        expires_at,
    })
}
fn check_point(point: &Point) -> Result<(), Failure> {
    if point.x > 8191 || point.y > 8191 {
        return Err(state::failure("invalid_request", "Desktop coordinates exceed the supported display size"));
    }
    Ok(())
}
async fn ready(agent: &Agent) -> Result<std::sync::Arc<state::State>, Failure> {
    state::native(agent, "Desktop automation")?;
    let state = state::get(agent)?;
    state::require(&state, "desktop")?;
    state::ready(agent, super::defaults::DESKTOP_PORT).await?;
    Ok(state)
}
pub(crate) async fn screenshot(agent: &Agent, claims: &Claims) -> Result<Vec<u8>, Failure> {
    let state = ready(agent).await?;
    let path = state.directory.path().join(format!("screen-{}.png", uuid::Uuid::new_v4()));
    let result =
        command::run(agent, claims, vec!["scrot".into(), "--silent".into(), path.to_string_lossy().into_owned()], None)
            .await;
    if let Err(error) = result {
        let _ = tokio::fs::remove_file(&path).await;
        return Err(error);
    }
    command::image(&path).await
}
pub(crate) async fn move_to(agent: &Agent, claims: &Claims, point: Point) -> Result<(), Failure> {
    ready(agent).await?;
    check_point(&point)?;
    command::run(
        agent,
        claims,
        vec!["xdotool".into(), "mousemove".into(), "--sync".into(), point.x.to_string(), point.y.to_string()],
        None,
    )
    .await?;
    Ok(())
}
pub(crate) async fn click(agent: &Agent, claims: &Claims, point: Point) -> Result<(), Failure> {
    ready(agent).await?;
    check_point(&point)?;
    // X11 button 1 is the primary pointer button (X protocol, pointer mapping).
    command::run(
        agent,
        claims,
        vec![
            "xdotool".into(),
            "mousemove".into(),
            "--sync".into(),
            point.x.to_string(),
            point.y.to_string(),
            "click".into(),
            "1".into(),
        ],
        None,
    )
    .await?;
    Ok(())
}
pub(crate) async fn text(agent: &Agent, claims: &Claims, text: Text) -> Result<(), Failure> {
    ready(agent).await?;
    if text.text.contains('\0') || text.text.len() > crate::contract::CAPTURE_BYTES {
        return Err(state::failure("invalid_request", "Invalid desktop text"));
    }
    command::run(
        agent,
        claims,
        vec![
            "xdotool".into(),
            "type".into(),
            "--clearmodifiers".into(),
            "--delay".into(),
            "0".into(),
            "--file".into(),
            "-".into(),
        ],
        Some(text.text.as_bytes()),
    )
    .await?;
    Ok(())
}
pub(crate) async fn key(agent: &Agent, claims: &Claims, key: Key) -> Result<(), Failure> {
    ready(agent).await?;
    if key.combo.is_empty()
        || key.combo.len() > 128
        || !key.combo.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'+')
    {
        return Err(state::failure("invalid_request", "Invalid xdotool key combination"));
    }
    command::run(agent, claims, vec!["xdotool".into(), "key".into(), "--clearmodifiers".into(), key.combo], None)
        .await?;
    Ok(())
}
pub(crate) async fn scroll(agent: &Agent, claims: &Claims, scroll: Scroll) -> Result<(), Failure> {
    ready(agent).await?;
    if scroll.dx.unsigned_abs() > 100 || scroll.dy.unsigned_abs() > 100 {
        return Err(state::failure("invalid_request", "Scroll exceeds its action budget"));
    }
    // X11 wheel mapping: buttons 4/5 vertical, 6/7 horizontal, documented by xdotool(1).
    for (value, negative, positive) in [(scroll.dy, "4", "5"), (scroll.dx, "6", "7")] {
        if value != 0 {
            command::run(
                agent,
                claims,
                vec![
                    "xdotool".into(),
                    "click".into(),
                    "--repeat".into(),
                    value.unsigned_abs().to_string(),
                    if value < 0 { negative.into() } else { positive.into() },
                ],
                None,
            )
            .await?;
        }
    }
    Ok(())
}
