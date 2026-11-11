// SPDX-License-Identifier: FSL-1.1-ALv2
//! Private tool paths outlive individual calls but never enter data archives. The process
//! supervisor keeps this opaque state without depending on the service implementations.
use super::types::MountView;
use crate::{
    auth::Claims,
    context::{Agent, Failure},
};
use axum::http::StatusCode;
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use tokio::sync::Mutex;
pub(crate) struct MountEntry {
    pub(crate) view: MountView,
    pub(crate) process: Arc<crate::process::Process>,
    pub(crate) fingerprint: Vec<u8>,
    pub(crate) _directory: std::fs::File,
}

pub(crate) struct State {
    pub(crate) directory: tempfile::TempDir,
    pub(crate) kind: Option<String>,
    pub(crate) access: Mutex<()>,
    pub(crate) ssh: Mutex<BTreeMap<String, Arc<dyn std::any::Any + Send + Sync>>>,
    pub(crate) mounts: Mutex<Vec<MountEntry>>,
    pub(crate) mounts_ready: std::sync::atomic::AtomicBool,
}
pub(crate) fn failure(code: &'static str, message: &str) -> Failure {
    Failure(StatusCode::CONFLICT, code, message.into())
}
pub(crate) fn native(agent: &Agent, capability: &str) -> Result<(), Failure> {
    if let Some(tier) = agent.processes.kernel_unsupported_tier() {
        return Err(Failure(
            StatusCode::NOT_IMPLEMENTED,
            "unsupported",
            if tier == "process" {
                format!(
                    "{capability} requires a verified process tool bundle and its private transport; use a supported container template or a release providing native process tools"
                )
            } else {
                format!("{capability} is unsupported on the {tier} tier; use a supported native container template")
            },
        ));
    }
    Ok(())
}
pub(crate) fn get(agent: &Agent) -> Result<Arc<State>, Failure> {
    let mut held = agent.processes.tools.state.lock().unwrap();
    if let Some(state) = held.as_ref() {
        return Ok(state.clone().downcast::<State>().expect("native tool state"));
    }
    let directory = tempfile::Builder::new().prefix("pryx-tools-").tempdir_in("/tmp")?;
    let data = std::fs::canonicalize(&agent.processes.data)?;
    if std::fs::canonicalize(directory.path())?.starts_with(data) {
        return Err(failure("unsupported", "Native tools require temporary storage outside persistent data"));
    }
    let kind = std::env::var(super::defaults::KIND_ENV).ok();
    let state = Arc::new(State {
        directory,
        kind,
        access: Mutex::new(()),
        ssh: Mutex::new(BTreeMap::new()),
        mounts: Mutex::new(Vec::new()),
        mounts_ready: std::sync::atomic::AtomicBool::new(false),
    });
    *held = Some(state.clone());
    Ok(state)
}
pub(crate) fn prepare(agent: &Agent) -> Result<(), Failure> {
    if agent.processes.kernel_unsupported_tier().is_some()
        || (std::env::var_os(super::defaults::KIND_ENV).is_none()
            && std::env::var_os(crate::contract::MOUNTS_ENV).is_none())
    {
        return Ok(());
    }
    let state = get(agent)?;
    let mut env = BTreeMap::from([
        (super::defaults::DIRECTORY_ENV.into(), state.directory.path().to_string_lossy().into_owned()),
        ("PRYX_DESKTOP_PORT".into(), super::defaults::DESKTOP_PORT.to_string()),
        ("PRYX_VNC_PORT".into(), super::defaults::VNC_PORT.to_string()),
        ("PRYX_CDP_PORT".into(), super::defaults::CDP_PORT.to_string()),
        ("PRYX_BROWSER_PORT".into(), super::defaults::BROWSER_PORT.to_string()),
    ]);
    if matches!(state.kind.as_deref(), Some("desktop" | "browser")) {
        std::fs::create_dir(state.directory.path().join("home"))?;
        env.insert("HOME".into(), state.directory.path().join("home").to_string_lossy().into_owned());
    }
    if state.kind.as_deref() == Some("desktop") {
        // X11 display syntax is documented by Xserver(1); TCP listening stays disabled.
        env.insert("DISPLAY".into(), ":10".into());
        env.insert("XAUTHORITY".into(), state.directory.path().join("Xauthority").to_string_lossy().into_owned());
    }
    *agent.processes.tools.environment.write().unwrap() = env;
    Ok(())
}
pub(crate) fn expiry(claims: &Claims) -> Result<i64, Failure> {
    let expires = claims.expires_at.min(crate::telemetry::now() + crate::auth::TOKEN_LIFETIME_SECONDS);
    if expires <= crate::telemetry::now() {
        return Err(Failure(StatusCode::UNAUTHORIZED, "expired", "Tool access expired".into()));
    }
    Ok(expires)
}
pub(crate) fn require(state: &State, kind: &str) -> Result<(), Failure> {
    if state.kind.as_deref() != Some(kind) {
        return Err(Failure(
            StatusCode::NOT_IMPLEMENTED,
            "unsupported",
            format!("Use the {kind} template for this tool"),
        ));
    }
    Ok(())
}
pub(crate) async fn ready(agent: &Agent, port: u16) -> Result<(), Failure> {
    let wait = async {
        loop {
            if tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port)).await.is_ok() {
                return Ok(());
            }
            if !agent.processes.list().iter().any(|process| process.running) {
                return Err(failure("tool_died", "Template service exited before readiness"));
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    };
    tokio::time::timeout(Duration::from_secs(30), wait)
        .await
        .map_err(|_| failure("tool_not_ready", "Template service did not become ready; check host tool requirements"))?
}
pub(crate) fn nonce() -> Result<String, Failure> {
    use ring::rand::SecureRandom;
    let mut bytes = [0; 32];
    ring::rand::SystemRandom::new().fill(&mut bytes).map_err(|_| failure("entropy", "Cannot create tool access"))?;
    use base64::Engine;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes))
}
pub(crate) fn write(state: &State, name: &str, value: &serde_json::Value) -> Result<(), Failure> {
    crate::paths::DataDir::open(state.directory.path())?.write_atomic(
        name,
        &serde_json::to_vec(value).expect("tool metadata serializes"),
        false,
    )?;
    Ok(())
}

pub(crate) async fn wait_mounts(agent: &Agent) -> Result<(), Failure> {
    if std::env::var_os(crate::contract::MOUNTS_ENV).is_none() {
        return Ok(());
    }
    let state = get(agent)?;
    tokio::time::timeout(Duration::from_secs(60), async {
        while !state.mounts_ready.load(std::sync::atomic::Ordering::Acquire) {
            tokio::time::sleep(Duration::from_millis(25)).await
        }
    })
    .await
    .map_err(|_| failure("mount_timeout", "Initial mounts were not delivered before the entrypoint deadline"))
}

pub(crate) fn mounts_pending(agent: &Agent) -> bool {
    if std::env::var_os(crate::contract::MOUNTS_ENV).is_none() {
        return false;
    }
    let held = agent.processes.tools.state.lock().unwrap();
    !held
        .as_ref()
        .and_then(|s| s.downcast_ref::<State>())
        .is_some_and(|s| s.mounts_ready.load(std::sync::atomic::Ordering::Acquire))
}
