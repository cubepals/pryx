// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Agent bootstrap and short-lived capabilities. Private signing material stays in the runtime's
//! secret environment; neither records, labels nor a workload view can contain its value.

use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;

use pryx_agent::auth::{Claims, SigningKey, TOKEN_LIFETIME_SECONDS};
use pryx_agent::contract::{AGENT_PORT_NAME, INTERNAL_PORT_PREFIX, KEY_LABEL, MAX_SPARE_PORTS, SECRET_ENV_PREFIX};

use super::*;
use crate::protocol::{
    AgentPort, AgentPortAssignment, AgentTokenRequest, AgentTokenResponse, AgentView, PortSpec, Proto,
};
use crate::runtime::AgentBoot;

impl Manager {
    pub(super) fn agent_binary(&self) -> Option<PathBuf> {
        if !self.runtime.supports_agent() {
            return None;
        }
        if self.runtime.is_wasm() {
            return std::env::current_exe().ok();
        }
        let path = &self.config.workloads.agent_binary;
        let metadata = std::fs::symlink_metadata(path).ok()?;
        if !metadata.is_file()
            || metadata.mode() & 0o022 != 0
            || metadata.mode() & 0o111 == 0
            || (metadata.uid() != rustix::process::geteuid().as_raw()
                && !(self.runtime.is_process() && metadata.uid() == 0))
        {
            return None;
        }
        // Writable ancestors would let another user replace a checked executable by path.
        for parent in path.ancestors().skip(1) {
            let metadata = std::fs::symlink_metadata(parent).ok()?;
            if !metadata.is_dir()
                || metadata.mode() & 0o022 != 0
                || (metadata.uid() != rustix::process::geteuid().as_raw()
                    && !(self.runtime.is_process() && metadata.uid() == 0))
            {
                return None;
            }
        }
        Some(path.clone())
    }

    pub(super) fn requested_ports(&self, spec: &WorkloadSpec) -> Vec<PortSpec> {
        let mut ports = spec.ports.clone();
        if self.agent_binary().is_some() {
            let mut candidate = pryx_agent::contract::AGENT_PORT;
            for slot in 0..=self.spare_count(spec) {
                while ports.iter().any(|p| p.protocol == Proto::Tcp && p.container_port == candidate) {
                    candidate += 1;
                }
                ports.push(PortSpec {
                    name: if slot == 0 {
                        AGENT_PORT_NAME.into()
                    } else {
                        format!("{INTERNAL_PORT_PREFIX}spare_{slot}")
                    },
                    container_port: candidate,
                    protocol: Proto::Tcp,
                    audience: vec![Audience::Edge],
                });
            }
        }
        ports
    }

    pub(super) fn spare_count(&self, spec: &WorkloadSpec) -> u16 {
        spec.spare_ports.unwrap_or(self.config.workloads.default_spare_ports).min(MAX_SPARE_PORTS)
    }

    pub(super) fn agent_boot(
        &self,
        record: &WorkloadRecord,
        spec: &WorkloadSpec,
    ) -> Result<Option<(AgentBoot, String)>, NodeError> {
        let Some(binary) = self.agent_binary() else { return Ok(None) };
        let encoded = super::agent_keys::load(&self.config.state_dir, &record.id, record.epoch.unwrap_or_default())
            .map_err(|source| NodeError::Io { what: "sandbox signing material".into(), source })?;
        let ports = self.requested_ports(spec);
        let agent_port = ports
            .iter()
            .find(|p| p.name == AGENT_PORT_NAME)
            .expect("requested_ports includes the agent")
            .container_port;
        let spare_ports = ports
            .iter()
            .filter(|p| p.name.starts_with(INTERNAL_PORT_PREFIX) && p.name != AGENT_PORT_NAME)
            .map(|p| p.container_port)
            .collect();
        Ok(Some((
            AgentBoot {
                public_origin: self.config.workloads.agent_public_origin.clone(),
                binary,
                sandbox: record.id.to_string(),
                epoch: record.epoch.unwrap_or_default(),
                secret_env: format!("{SECRET_ENV_PREFIX}{}", uuid::Uuid::new_v4().simple()),
                domain: spec.agent.as_ref().map_or_else(|| "invalid".into(), |a| a.domain.clone()),
                agent_port,
                spare_ports,
                keep_alive: spec.agent.as_ref().is_some_and(|a| a.keep_alive),
            },
            encoded,
        )))
    }

    pub(super) fn agent_view(&self, record: &WorkloadRecord, info: Option<&ContainerInfo>) -> Option<AgentView> {
        let info = info?;
        let key_name = info.labels.get(KEY_LABEL)?;
        let key = SigningKey::from_encoded(info.env.get(key_name)?).ok()?;
        let port = |p: &AllocatedPort| AgentPort {
            port: if self.runtime.uses_host_ports() { p.host_port } else { p.container_port },
            host_port: p.host_port,
            endpoints: self.config.network.edge_ips.iter().map(|ip| endpoint(ip, p.host_port)).collect(),
        };
        let data = pryx_agent::paths::DataDir::open(&self.runtime.data_path(&self.store.data_dir(&record.id))).ok();
        let ports: pryx_agent::state::PortState = data
            .as_ref()
            .and_then(|d| pryx_agent::state::read(d, pryx_agent::state::PORTS_PATH).ok())
            .unwrap_or_default();
        let activity: pryx_agent::state::Activity = data
            .as_ref()
            .and_then(|d| pryx_agent::state::read(d, pryx_agent::state::ACTIVITY_PATH).ok())
            .unwrap_or_default();
        let spares: Vec<_> = record
            .ports
            .iter()
            .filter(|p| p.name.starts_with(INTERNAL_PORT_PREFIX) && p.name != AGENT_PORT_NAME)
            .map(port)
            .collect();
        let exposed = ports
            .targets
            .iter()
            .zip(&spares)
            .filter_map(|(target, spare)| target.map(|port| AgentPortAssignment { port, slot: spare.port }))
            .collect();
        Some(AgentView {
            public_key: key.public_key(),
            port: port(record.ports.iter().find(|p| p.name == AGENT_PORT_NAME)?),
            spares,
            port_revision: ports.revision.min(pryx_agent::state::MAX_REVISION),
            exposed,
            activity_at: activity.at.clamp(0, time::OffsetDateTime::now_utc().unix_timestamp()),
            active_streams: activity.streams,
        })
    }

    pub async fn agent_token(
        &self,
        id: WorkloadId,
        epoch: Option<u64>,
        request: AgentTokenRequest,
    ) -> Result<AgentTokenResponse, NodeError> {
        let lock = self.lock_for(&id);
        let _guard = lock.lock().await;
        let record = self.record(&id)?;
        check_epoch(&record, epoch, EpochRule::Exact)?;
        let info = self.observe(&record).await?;
        let agent = self.agent_view(&record, info.as_ref()).ok_or_else(|| NodeError::Conflict {
            code: ConflictCode::NoCompute,
            message: "This workload has no agent".into(),
        })?;
        let info = info.as_ref().expect("agent_view requires runtime info");
        let key = info
            .labels
            .get(KEY_LABEL)
            .and_then(|name| info.env.get(name))
            .and_then(|encoded| SigningKey::from_encoded(encoded).ok())
            .ok_or_else(|| NodeError::Internal("Cannot issue sandbox token".into()))?;
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        let expires_at = request.expires_at.unwrap_or(now + TOKEN_LIFETIME_SECONDS).min(now + TOKEN_LIFETIME_SECONDS);
        if expires_at <= now {
            return Err(NodeError::Invalid(vec![crate::protocol::FieldError {
                field: "expiresAt".into(),
                problem: "must be in the future".into(),
            }]));
        }
        let epoch = record.epoch.unwrap_or_default();
        let claims = Claims {
            sandbox: id.to_string(),
            epoch,
            issued_at: now,
            expires_at,
            scope: request.scope,
            max_ports: request.max_ports.min(agent.spares.len() as u16),
            max_exec_seconds: request.max_exec_seconds,
            file: None,
        };
        let token = key.sign(&claims).map_err(|_| NodeError::Internal("Cannot issue sandbox token".into()))?;
        Ok(AgentTokenResponse { token, expires_at, epoch, agent })
    }
}
