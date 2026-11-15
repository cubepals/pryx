// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2
//! Authorize the existing fixed agent file operations over the owned private endpoint.
//! Endpoint/signing policy stays here; cancellation-owned cleanup remains in the parent.
use super::*;
use crate::http_client;
use hyper::{Response, body::Incoming};
use pryx_agent::auth::{Claims, Scope, TOKEN_LIFETIME_SECONDS};

pub(super) struct Agent {
    endpoint: SocketAddr,
    key: SigningKey,
    sandbox: String,
    epoch: u64,
    timeout: Duration,
}

impl Agent {
    pub(super) fn new(
        endpoint: SocketAddr,
        key: SigningKey,
        identity: (String, u64),
        timeout: Duration,
    ) -> Result<Self, RuntimeError> {
        let private = match endpoint.ip() {
            std::net::IpAddr::V4(ip) => ip.is_loopback() || ip.is_private(),
            std::net::IpAddr::V6(ip) => ip.is_loopback() || ip.is_unique_local(),
        };
        if !private || endpoint.port() == 0 || timeout.is_zero() {
            return Err(RuntimeError::Other("File service requires a private endpoint and positive deadline".into()));
        }
        Ok(Self { endpoint, key, sandbox: identity.0, epoch: identity.1, timeout })
    }
}

#[async_trait]
impl Transport for Agent {
    async fn request(
        &self,
        operation: Operation,
        path: &str,
        body: http_client::Body,
    ) -> Result<Response<Incoming>, RuntimeError> {
        let (route, method) = operation.route();
        let mut url = reqwest::Url::parse(&format!("http://{}/_pryx/files/{route}", self.endpoint))
            .map_err(|_| RuntimeError::Other("Invalid file-service endpoint".into()))?;
        url.query_pairs_mut().append_pair("path", path);
        let now = crate::clock::now().unix_timestamp();
        let token = self
            .key
            .sign(&Claims {
                sandbox: self.sandbox.clone(),
                epoch: self.epoch,
                issued_at: now,
                expires_at: now + TOKEN_LIFETIME_SECONDS,
                scope: Scope::Sandbox,
                max_ports: 0,
                max_exec_seconds: None,
                file: None,
            })
            .map_err(|_| RuntimeError::Other("Cannot authorize file service".into()))?;
        let path = format!("{}?{}", url.path(), url.query().unwrap_or_default());
        http_client::local_stream(
            self.endpoint,
            method,
            &path,
            &[("authorization", format!("Bearer {token}"))],
            body,
            self.timeout,
        )
        .await
        .map_err(|problem| RuntimeError::Other(problem.to_string()))
    }
}
