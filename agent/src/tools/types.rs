// SPDX-License-Identifier: FSL-1.1-ALv2
//! Native tool schemas carry public metadata only. Client private SSH keys appear solely in
//! the one creation response; mount views contain no credentials or signed URLs.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ToolAccess {
    pub url: String,
    pub expires_at: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Point {
    pub x: u32,
    pub y: u32,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Text {
    pub text: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Key {
    pub combo: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Scroll {
    pub dx: i32,
    pub dy: i32,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct BrowserShot {
    pub url: String,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct SshView {
    pub id: String,
    pub port: u16,
    pub slot: u16,
    pub user: String,
    pub host_key: String,
    pub expires_at: i64,
    pub running: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub web_socket: Option<bool>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct SshCreated {
    #[serde(flatten)]
    pub view: SshView,
    pub key: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct SshExpiry {
    pub expires_at: i64,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct MountCredentials {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub session_token: Option<String>,
    pub endpoint: Option<String>,
    pub region: Option<String>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct MountSpec {
    pub bucket: String,
    pub path: String,
    #[serde(default)]
    pub read_only: bool,
    pub credentials: MountCredentials,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct MountView {
    pub bucket: String,
    pub path: String,
    pub read_only: bool,
    pub backend: String,
    pub state: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct MountsInit {
    pub mounts: Vec<MountSpec>,
}
