// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Optional agent bootstrap and capabilities. Signing material is never a wire type. Features
//! gate new requests; old records omit all default fields and retain their original digest.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentOptions {
    pub(crate) domain: String,
    #[serde(default)]
    pub(crate) keep_alive: bool,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum NetworkMode {
    #[default]
    Internet,
    None,
}

impl NetworkMode {
    pub(crate) fn is_default(&self) -> bool {
        *self == Self::Internet
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct AgentPort {
    pub port: u16,
    pub host_port: u16,
    pub endpoints: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct AgentView {
    pub public_key: String,
    pub port: AgentPort,
    pub spares: Vec<AgentPort>,
    #[serde(default)]
    pub port_revision: u64,
    #[serde(default)]
    pub exposed: Vec<AgentPortAssignment>,
    #[serde(default)]
    pub activity_at: i64,
    #[serde(default)]
    pub active_streams: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct AgentPortAssignment {
    pub port: u16,
    pub slot: u16,
}

#[derive(Clone, Debug, Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentTokenRequest {
    pub scope: pryx_agent::auth::Scope,
    pub max_ports: u16,
    pub max_exec_seconds: Option<u32>,
    pub expires_at: Option<i64>,
}

#[derive(Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct AgentTokenResponse {
    pub token: String,
    pub expires_at: i64,
    pub epoch: u64,
    pub agent: AgentView,
}

pub use pryx_agent::paths::FileInfo;

#[derive(Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct FileList {
    pub files: Vec<FileInfo>,
}
