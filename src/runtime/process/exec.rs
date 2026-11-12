// SPDX-License-Identifier: FSL-1.1-ALv2
//! Legacy node exec uses the trusted supervisor too; it cannot bypass the process tier's
//! filesystem, syscall or network policy. The mutation is submitted once.
use super::journal::error;
use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};
use pryx_agent::auth::{Claims, Scope, SigningKey};
const MAX_SCOPE_RESPONSE_BYTES: usize = 4096;

impl ProcessRuntime {
    async fn raw(
        &self,
        name: &str,
        method: hyper::Method,
        path: &str,
        body: serde_json::Value,
        timeout: Duration,
        nonce: Option<&str>,
    ) -> Result<Bytes, RuntimeError> {
        let entry = self.entry(name)?;
        let boot = entry.spec.agent.as_ref().ok_or_else(|| error("Missing process agent"))?;
        let key = entry
            .spec
            .env
            .iter()
            .find(|(key, _)| key == &boot.secret_env)
            .and_then(|(_, key)| SigningKey::from_encoded(key).ok())
            .ok_or_else(|| error("Missing process signing material"))?;
        let port = entry
            .spec
            .ports
            .iter()
            .find(|p| p.container_port == boot.agent_port)
            .ok_or_else(|| error("Missing process endpoint"))?
            .host_port;
        let now = crate::clock::now().unix_timestamp();
        let token = key
            .sign(&Claims {
                sandbox: boot.sandbox.clone(),
                epoch: boot.epoch,
                issued_at: now,
                expires_at: now + pryx_agent::auth::TOKEN_LIFETIME_SECONDS,
                scope: Scope::Sandbox,
                max_ports: boot.spare_ports.len() as u16,
                max_exec_seconds: None,
                file: None,
            })
            .map_err(error)?;
        let mut headers = vec![
            ("authorization", format!("Bearer {token}")),
            ("content-type", "application/json".into()),
            ("Idempotency-Key", uuid::Uuid::new_v4().to_string()),
        ];
        if let Some(nonce) = nonce {
            headers.push((pryx_agent::process_observation::NONCE_HEADER, nonce.into()));
        }
        let response = crate::http_client::send(
            method,
            &format!("http://127.0.0.1:{port}{path}"),
            &headers,
            crate::http_client::full(serde_json::to_vec(&body).map_err(error)?),
            None,
            timeout,
        )
        .await
        .map_err(error)?;
        let status = response.status();
        let bytes = tokio::time::timeout(
            timeout,
            crate::http_client::read_body(
                response,
                if nonce.is_some() { MAX_SCOPE_RESPONSE_BYTES } else { pryx_agent::contract::MAX_REQUEST_BYTES * 2 },
            ),
        )
        .await
        .map_err(error)?
        .map_err(error)?;
        if !status.is_success() {
            return Err(error(format!("Process agent refused {status}")));
        }
        Ok(bytes)
    }
    async fn request(
        &self,
        name: &str,
        method: hyper::Method,
        path: &str,
        body: serde_json::Value,
        timeout: Duration,
    ) -> Result<serde_json::Value, RuntimeError> {
        let bytes = self.raw(name, method, path, body, timeout, None).await?;
        if bytes.is_empty() { Ok(serde_json::Value::Null) } else { serde_json::from_slice(&bytes).map_err(error) }
    }
    pub(super) async fn agent_scope_proven(&self, entry: &Entry) -> bool {
        let Some(pid) = observe::matching_pid(entry.pid, entry.ticks) else { return false };
        let Some(boot) = &entry.spec.agent else { return false };
        let Some(key) = entry
            .spec
            .env
            .iter()
            .find(|(name, _)| name == &boot.secret_env)
            .and_then(|(_, value)| SigningKey::from_encoded(value).ok())
        else {
            return false;
        };
        let nonce = uuid::Uuid::new_v4().to_string();
        let Ok(response) = self
            .raw(
                &entry.spec.name,
                hyper::Method::GET,
                "/_pryx/health",
                serde_json::Value::Null,
                super::super::ENGINE_PROBE_TIMEOUT,
                Some(&nonce),
            )
            .await
        else {
            return false;
        };
        let Ok(response) = serde_json::from_slice::<serde_json::Value>(&response) else { return false };
        let Some(proof) = response
            .get("processObservation")
            .cloned()
            .and_then(|value| serde_json::from_value::<pryx_agent::process_observation::Proof>(value).ok())
        else {
            return false;
        };
        observe::matching_pid(entry.pid, entry.ticks) == Some(pid)
            && pryx_agent::process_observation::verify(
                &proof,
                &key.public_key(),
                &nonce,
                &boot.sandbox,
                boot.epoch,
                pid,
            )
    }
    pub(super) async fn agent_health(&self, name: &str) -> Result<(), RuntimeError> {
        self.request(name, hyper::Method::GET, "/_pryx/health", serde_json::Value::Null, Duration::from_millis(100))
            .await
            .map(|_| ())
    }
    pub(super) async fn agent_exec(
        &self,
        name: &str,
        command: &[String],
        timeout: Duration,
        limit: usize,
    ) -> Result<ExecOutput, RuntimeError> {
        let started = self
            .request(
                name,
                hyper::Method::POST,
                "/_pryx/processes",
                serde_json::json!({"command":command,"timeout":timeout.as_secs().max(1)}),
                timeout,
            )
            .await?;
        let id = started.get("id").and_then(|v| v.as_str()).ok_or_else(|| error("Process response omitted ID"))?;
        let waited = self
            .request(
                name,
                hyper::Method::GET,
                &format!("/_pryx/processes/{id}/wait"),
                serde_json::Value::Null,
                timeout + Duration::from_secs(1),
            )
            .await?;
        // The public compatibility transport consumes live NDJSON. This fallback retrieves the
        // bounded replay by the same API and never resubmits an uncertain command.
        let mut result = ExecOutput {
            exit_code: waited.get("code").and_then(serde_json::Value::as_i64),
            timed_out: waited.get("timedOut").and_then(serde_json::Value::as_bool).unwrap_or(false),
            ..Default::default()
        };
        let replay = self
            .raw(
                name,
                hyper::Method::GET,
                &format!("/_pryx/processes/{id}/output"),
                serde_json::Value::Null,
                timeout,
                None,
            )
            .await?;
        for line in replay.split(|byte| *byte == b'\n').filter(|line| !line.is_empty()) {
            let event: serde_json::Value = serde_json::from_slice(line).map_err(error)?;
            if event["type"] == "output" {
                let decoded = STANDARD.decode(event["bytes"].as_str().unwrap_or_default()).map_err(error)?;
                let (buffer, truncated) = if event["stream"] == "stderr" {
                    (&mut result.stderr, &mut result.stderr_truncated)
                } else {
                    (&mut result.stdout, &mut result.stdout_truncated)
                };
                let room = limit.saturating_sub(buffer.len());
                *truncated |= decoded.len() > room;
                buffer.extend_from_slice(&decoded[..decoded.len().min(room)]);
            }
        }
        result.killed = waited.get("killed").and_then(serde_json::Value::as_bool).unwrap_or(false);
        Ok(result)
    }
}
