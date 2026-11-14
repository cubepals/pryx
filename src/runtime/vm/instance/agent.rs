// SPDX-License-Identifier: FSL-1.1-ALv2
//! Compatibility exec and bounded reads use the authenticated guest agent over its owned
//! private forwarding process. Tokens are minted in memory and redirects are never followed.
use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};
use hyper::Method;
use pryx_agent::auth::{Claims, Scope, SigningKey, TOKEN_LIFETIME_SECONDS};
use serde_json::{Value, json};

pub(super) fn key(spec: &ContainerSpec) -> Result<SigningKey, RuntimeError> {
    let boot = spec.agent.as_ref().ok_or_else(|| error("VM agent bootstrap is missing"))?;
    spec.env
        .iter()
        .find(|(name, _)| name == &boot.secret_env)
        .and_then(|(_, value)| SigningKey::from_encoded(value).ok())
        .ok_or_else(|| error("VM signing material is missing"))
}
pub(super) fn endpoint(spec: &ContainerSpec) -> Result<std::net::SocketAddr, RuntimeError> {
    let boot = spec.agent.as_ref().ok_or_else(|| error("VM agent bootstrap is missing"))?;
    let binding = spec
        .ports
        .iter()
        .find(|port| port.container_port == boot.agent_port && port.protocol == crate::protocol::Proto::Tcp)
        .ok_or_else(|| error("VM agent binding is missing"))?;
    let ip = binding.host_ips.first().ok_or_else(|| error("VM agent private address is missing"))?;
    Ok(std::net::SocketAddr::new(*ip, binding.host_port))
}
fn headers(spec: &ContainerSpec, scope: Scope) -> Result<[(&'static str, String); 3], RuntimeError> {
    let boot = spec.agent.as_ref().ok_or_else(|| error("VM agent bootstrap is missing"))?;
    let now = crate::clock::now().unix_timestamp();
    let token = key(spec)?
        .sign(&Claims {
            sandbox: boot.sandbox.clone(),
            epoch: boot.epoch,
            issued_at: now,
            expires_at: now + TOKEN_LIFETIME_SECONDS,
            scope,
            max_ports: boot.spare_ports.len() as u16,
            max_exec_seconds: None,
            file: None,
        })
        .map_err(error)?;
    Ok([
        ("authorization", format!("Bearer {token}")),
        ("content-type", "application/json".into()),
        (pryx_agent::contract::IDEMPOTENCY_HEADER, uuid::Uuid::new_v4().to_string()),
    ])
}
pub(super) async fn raw(
    spec: &ContainerSpec,
    method: Method,
    path: &str,
    body: Value,
    timeout: Duration,
) -> Result<Vec<u8>, RuntimeError> {
    let endpoint = endpoint(spec)?;
    let scope = if method == Method::GET && matches!(path, "/_pryx/health" | "/_pryx/ports") {
        Scope::Routes
    } else {
        Scope::Sandbox
    };
    let headers = headers(spec, scope)?;
    let response = crate::http_client::send(
        method,
        &format!("http://{endpoint}{path}"),
        &headers,
        crate::http_client::full(serde_json::to_vec(&body).map_err(error)?),
        None,
        timeout,
    )
    .await
    .map_err(error)?;
    let status = response.status();
    let bytes =
        tokio::time::timeout(timeout, crate::http_client::read_body(response, pryx_agent::contract::CAPTURE_BYTES * 4))
            .await
            .map_err(|_| RuntimeError::Timeout("VM agent response exceeded its deadline".into()))?
            .map_err(error)?;
    if !status.is_success() {
        return Err(error(format!("VM agent refused {status}")));
    }
    Ok(bytes.to_vec())
}
pub(super) async fn log_response(
    spec: &ContainerSpec,
    follow: bool,
) -> Result<hyper::Response<hyper::body::Incoming>, RuntimeError> {
    let headers = headers(spec, Scope::Sandbox)?;
    tokio::time::timeout(
        crate::runtime::ENGINE_PROBE_TIMEOUT,
        crate::http_client::local_stream(
            endpoint(spec)?,
            Method::GET,
            if follow { "/_pryx/logs?follow=true" } else { "/_pryx/logs" },
            &headers,
            crate::http_client::empty(),
            crate::runtime::ENGINE_PROBE_TIMEOUT,
        ),
    )
    .await
    .map_err(|_| RuntimeError::Timeout("VM log response headers exceeded their deadline".into()))?
    .map_err(|problem| RuntimeError::Unavailable(problem.to_string()))
}

pub(super) async fn snapshot(spec: &ContainerSpec) -> Result<pryx_agent::state::Snapshot, RuntimeError> {
    #[derive(serde::Deserialize)]
    struct Health {
        state: pryx_agent::state::Snapshot,
    }
    let health: Health = serde_json::from_slice(
        &raw(spec, Method::GET, "/_pryx/health", Value::Null, crate::runtime::ENGINE_PROBE_TIMEOUT).await?,
    )
    .map_err(error)?;
    Ok(health.state)
}

impl VmRuntime {
    pub(super) async fn execute(
        &self,
        name: &str,
        command: &[String],
        timeout: Duration,
        limit: usize,
    ) -> Result<crate::runtime::ExecOutput, RuntimeError> {
        let entry = self.entry(name)?;
        if !self.state.live.lock().unwrap().contains_key(&entry.id) {
            return Err(error("VM sandbox is stopped"));
        }
        let start = pryx_agent::process::Start {
            command: pryx_agent::process::Command::Argv(command.to_vec()),
            env: Default::default(),
            cwd: None,
            stdin: false,
            pty: false,
            cols: None,
            rows: None,
            timeout: Some(u32::try_from(timeout.as_secs().max(1)).map_err(error)?),
        };
        let started: Value = serde_json::from_slice(
            &raw(&entry.spec, Method::POST, "/_pryx/processes", serde_json::to_value(start).map_err(error)?, timeout)
                .await?,
        )
        .map_err(error)?;
        let id = started["id"].as_str().ok_or_else(|| error("VM process response omitted ID"))?;
        let waited: Value = serde_json::from_slice(
            &raw(
                &entry.spec,
                Method::GET,
                &format!("/_pryx/processes/{id}/wait"),
                Value::Null,
                timeout + Duration::from_secs(1),
            )
            .await?,
        )
        .map_err(error)?;
        let mut result = crate::runtime::ExecOutput {
            exit_code: waited["code"].as_i64(),
            timed_out: waited["timedOut"].as_bool().unwrap_or(false),
            killed: waited["killed"].as_bool().unwrap_or(false),
            stdout_truncated: waited["stdoutTruncated"].as_bool().unwrap_or(false),
            stderr_truncated: waited["stderrTruncated"].as_bool().unwrap_or(false),
            ..Default::default()
        };
        let output =
            raw(&entry.spec, Method::GET, &format!("/_pryx/processes/{id}/output"), json!(null), timeout).await?;
        for line in output.split(|b| *b == b'\n').filter(|line| !line.is_empty()) {
            let event: Value = serde_json::from_slice(line).map_err(error)?;
            if event["type"] == "output" {
                let decoded = STANDARD
                    .decode(event["bytes"].as_str().ok_or_else(|| error("VM output omitted bytes"))?)
                    .map_err(error)?;
                let (buffer, truncated) = if event["stream"] == "stderr" {
                    (&mut result.stderr, &mut result.stderr_truncated)
                } else {
                    (&mut result.stdout, &mut result.stdout_truncated)
                };
                let room = limit.saturating_sub(buffer.len());
                *truncated |= decoded.len() > room;
                buffer.extend_from_slice(&decoded[..room.min(decoded.len())]);
            }
        }
        Ok(result)
    }
}
