// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

//! What a workload is: the spec the control plane sends (`WorkloadSpec`), and the same spec as
//! pryxd keeps it (`SpecRecord`, secret values dropped), with the defaults a request may leave out.
//! Whether this host will run a spec is not decided here but in `spec/validate.rs`. What pryxd
//! reports about a workload once it exists is in `workload.rs`.
//!
//! Parts (`spec/`):
//! - `validate.rs`: decides whether a spec is one this host will run.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

mod validate;

pub(crate) use validate::*;

/// A secret value: never logged, never persisted by pryxd, never returned.
#[derive(Clone, Deserialize, PartialEq, Eq)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(transparent)]
pub(crate) struct Secret(String);

impl Secret {
    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[redacted]")
    }
}

/// The backend a workload asks for. An omitted choice retains the host's default.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum Isolation {
    Container,
    Process,
    Wasm,
}
impl Isolation {
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "container" => Some(Self::Container),
            "process" => Some(Self::Process),
            "wasm" => Some(Self::Wasm),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Container => "container",
            Self::Process => "process",
            Self::Wasm => "wasm",
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkloadSpec {
    /// Sent only after the node announces multi-tier.
    #[serde(default)]
    pub(crate) isolation: Option<Isolation>,
    /// An image reference. Must match the host's `allowed_images` prefixes.
    #[serde(default)]
    pub(crate) image: String,
    #[serde(default)]
    #[cfg_attr(test, schemars(length(max = super::MAX_WASM_PACKAGES)))]
    pub(crate) packages: Vec<String>,
    /// Replaces the image's entrypoint; absent, the image starts as it was built to.
    #[serde(default)]
    pub(crate) entrypoint: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) workdir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) egress: Option<EgressPolicy>,
    #[serde(default)]
    pub(crate) env: BTreeMap<String, String>,
    /// Environment the workload needs but nobody else may see.
    #[serde(default)]
    pub(crate) secrets: BTreeMap<String, Secret>,
    pub(crate) resources: Resources,
    pub(crate) storage: Storage,
    #[serde(default)]
    pub(crate) ports: Vec<PortSpec>,
    #[serde(default)]
    pub(crate) stop: StopSpec,
    #[serde(default)]
    pub restart: RestartSpec,
    #[serde(default)]
    pub(crate) labels: BTreeMap<String, String>,
    /// What a suspended workload may wake into without losing its memory: a `PUT` whose spec
    /// differs but names the same `resumesOn` as the spec it was suspended on keeps it suspended,
    /// and its spec is put in place at its next boot. Not part of the spec's digest.
    #[serde(default)]
    pub(crate) resumes_on: Option<String>,
    #[serde(default)]
    pub(crate) agent: Option<super::AgentOptions>,
    #[serde(default)]
    #[cfg_attr(test, schemars(range(max = pryx_agent::contract::MAX_SPARE_PORTS)))]
    pub(crate) spare_ports: Option<u16>,
    #[serde(default)]
    pub(crate) network: super::NetworkMode,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Resources {
    /// Hard memory limit (no swap). Expressed in MiB.
    pub(crate) memory_mb: u32,
    /// CPU ceiling in thousandths of a core (CFS quota). Also what the JVM counts as its
    /// processors. Absent: no ceiling, weight only.
    #[serde(default)]
    pub(crate) cpu_millis: Option<u32>,
    /// CPU reserved for admission, in thousandths of a core. This is not a CPU ceiling.
    /// Sent only after the node announces cpu-reservation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) cpu_reservation_millis: Option<u32>,
    /// Share of CPU under contention, relative (cgroup weight). Absent: proportional to memory.
    #[serde(default)]
    pub(crate) cpu_weight: Option<u32>,
    /// Processes and threads. Absent: the host default (4096).
    #[serde(default)]
    pub(crate) pids_limit: Option<u32>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Storage {
    /// Where the workload's persistent data appears inside it.
    pub(crate) mount_path: String,
    /// The size the control plane promised. Reported against measured usage; not enforced in v1
    /// (no per-directory quota on a plain filesystem).
    pub(crate) size_gb: u32,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum Proto {
    #[default]
    Tcp,
    Udp,
}

impl Proto {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Tcp => "tcp",
            Self::Udp => "udp",
        }
    }
}

/// Who must be able to reach a port: the edge (players, through the router) or the control
/// plane (console, status ping). The host decides which of its addresses each audience uses.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub(crate) enum Audience {
    Edge,
    Control,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PortSpec {
    pub(crate) name: String,
    pub(crate) container_port: u16,
    #[serde(default)]
    pub(crate) protocol: Proto,
    pub(crate) audience: Vec<Audience>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[allow(clippy::upper_case_acronyms, reason = "the names are the signals as the wire spells them")]
pub(crate) enum StopSignal {
    SIGTERM,
    SIGINT,
}

impl StopSignal {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::SIGTERM => "SIGTERM",
            Self::SIGINT => "SIGINT",
        }
    }
}

/// A field left out takes its value from `Default`, whether `stop` is absent or partial.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct StopSpec {
    /// Sent first; the workload saves and exits on it.
    pub(crate) signal: StopSignal,
    /// How long the workload gets after the signal before it is killed.
    pub(crate) timeout_seconds: u32,
}

impl Default for StopSpec {
    fn default() -> Self {
        Self { signal: StopSignal::SIGTERM, timeout_seconds: 90 }
    }
}

/// The longest `stop.timeoutSeconds` a spec may ask for.
pub(crate) const MAX_STOP_TIMEOUT: u32 = 600;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(rename_all = "kebab-case")]
pub(crate) enum RestartPolicy {
    /// Never started again by the host: a crash stays a crash until the control plane acts.
    No,
    /// Started again after a failing exit, up to `maxRetries` times (0 means never, not an unlimited retry count). Never after a requested stop.
    /// After a restart of the host, a workload that was running when the host went down is
    /// resumed; after a restart of pryxd alone, in the same boot, nothing is. In fleet mode
    /// either needs the execution lease, which the node takes only after applying the fences of
    /// the heartbeat answer that grants it.
    OnFailure,
}

/// A field left out takes its value from `Default`, whether `restart` is absent or partial.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct RestartSpec {
    pub(crate) policy: RestartPolicy,
    pub max_retries: u32,
}

impl Default for RestartSpec {
    fn default() -> Self {
        Self { policy: RestartPolicy::OnFailure, max_retries: 3 }
    }
}

impl WorkloadSpec {
    /// The spec as pryxd keeps it: secret values dropped, names kept.
    pub fn record(&self) -> SpecRecord {
        SpecRecord {
            isolation: self.isolation,
            image: self.image.clone(),
            packages: self.packages.clone(),
            entrypoint: self.entrypoint.clone(),
            workdir: self.workdir.clone(),
            user: self.user.clone(),
            egress: self.egress.clone(),
            env: self.env.clone(),
            secret_names: self.secrets.keys().cloned().collect(),
            resources: self.resources.clone(),
            storage: self.storage.clone(),
            ports: self.ports.clone(),
            stop: self.stop.clone(),
            restart: self.restart.clone(),
            labels: self.labels.clone(),
            agent: self.agent.clone(),
            spare_ports: self.spare_ports,
            network: self.network,
        }
    }
}

/// A spec without its secret values: what pryxd persists and puts in labels. Enough to
/// describe and rediscover a workload; not enough to recreate it, which only the control plane
/// can ask for, since only it holds the secrets.
///
/// Its JSON is what the digest hashes, and nodes keep the digest an older build computed: a spec
/// whose digest moves is a new spec, and its container is made again. So a field added within v1,
/// here or in a struct this one holds, carries `#[serde(default, skip_serializing_if = ...)]`
/// (`Option::is_none`, or a test for its default): a spec that doesn't use it serializes, and
/// hashes, exactly as before, and an older build still reads its record. Today's digests are
/// pinned in `the_digest_of_a_spec_is_the_same_in_every_build`.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SpecRecord {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) isolation: Option<Isolation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) packages: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) agent: Option<super::AgentOptions>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) spare_ports: Option<u16>,
    #[serde(default, skip_serializing_if = "super::NetworkMode::is_default")]
    pub(crate) network: super::NetworkMode,
    pub(crate) image: String,
    pub(crate) entrypoint: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) workdir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) egress: Option<EgressPolicy>,
    pub(crate) env: BTreeMap<String, String>,
    pub(crate) secret_names: Vec<String>,
    pub(crate) resources: Resources,
    pub(crate) storage: Storage,
    pub(crate) ports: Vec<PortSpec>,
    pub(crate) stop: StopSpec,
    pub(crate) restart: RestartSpec,
    pub(crate) labels: BTreeMap<String, String>,
}

impl SpecRecord {
    /// The spec again, with these secret values, and `resumes_on`: for a spec pryxd kept
    /// whose secrets it can take from the container they were given to. None if `secrets` lacks
    /// one of the names the spec holds.
    pub(crate) fn with_secrets(
        &self,
        secrets: &BTreeMap<String, String>,
        resumes_on: Option<String>,
    ) -> Option<WorkloadSpec> {
        let secrets = self
            .secret_names
            .iter()
            .map(|name| secrets.get(name).map(|value| (name.clone(), Secret(value.clone()))))
            .collect::<Option<BTreeMap<_, _>>>()?;
        Some(WorkloadSpec {
            isolation: self.isolation,
            image: self.image.clone(),
            packages: self.packages.clone(),
            entrypoint: self.entrypoint.clone(),
            workdir: self.workdir.clone(),
            user: self.user.clone(),
            egress: self.egress.clone(),
            env: self.env.clone(),
            secrets,
            resources: self.resources.clone(),
            storage: self.storage.clone(),
            ports: self.ports.clone(),
            stop: self.stop.clone(),
            restart: self.restart.clone(),
            labels: self.labels.clone(),
            resumes_on,
            agent: self.agent.clone(),
            spare_ports: self.spare_ports,
            network: self.network,
        })
    }

    /// `sha256:` of the canonical JSON (maps are ordered). Secret values never enter it; a
    /// changed secret is caught by comparing the container's environment instead.
    pub fn digest(&self) -> String {
        let canonical = serde_json::to_vec(self).expect("a spec record always serializes");
        format!("sha256:{}", hex::encode(Sha256::digest(canonical)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn example_spec() -> serde_json::Value {
        serde_json::json!({
            "image": "ghcr.io/example/http-app:1.0",
            "env": { "APP_MODE": "TRUE", "TYPE": "VANILLA", "VERSION": "1.21.8", "MEMORY": "2304M" },
            "secrets": { "APP_TOKEN": "hunter2hunter2" },
            "resources": { "memoryMb": 3072, "cpuMillis": 2000 },
            "storage": { "mountPath": "/data", "sizeGb": 5 },
            "ports": [
                { "name": "http", "containerPort": crate::examples::HTTP_PORT, "audience": ["edge", "control"] },
                { "name": "admin", "containerPort": crate::examples::ADMIN_PORT, "audience": ["control"] }
            ],
            "stop": { "signal": "SIGTERM", "timeoutSeconds": 90 },
            "labels": { "example.sandbox": "0b6f1f2e-6a47-4c9a-9a39-2f4ac7e51f10" }
        })
    }

    pub(super) fn spec(value: serde_json::Value) -> WorkloadSpec {
        serde_json::from_value(value).expect("parses")
    }

    #[test]
    fn isolation_is_optional_but_an_explicit_choice_survives_recovery() {
        let legacy = spec(example_spec()).record();
        assert!(serde_json::to_value(&legacy).unwrap().get("isolation").is_none());
        let mut value = example_spec();
        value["isolation"] = "process".into();
        let record = spec(value).record();
        assert_ne!(record.digest(), legacy.digest());
        assert_eq!(record.isolation, Some(Isolation::Process));
        let secrets = BTreeMap::from([("APP_TOKEN".into(), "private".into())]);
        assert_eq!(record.with_secrets(&secrets, None).unwrap().isolation, Some(Isolation::Process));
        let mut value = example_spec();
        value["isolation"] = "unknown".into();
        assert!(serde_json::from_value::<WorkloadSpec>(value).is_err());
    }

    #[test]
    fn unknown_fields_are_refused_not_ignored() {
        let mut v = example_spec();
        v["privileged"] = serde_json::json!(true);
        assert!(serde_json::from_value::<WorkloadSpec>(v).is_err());
        let mut v = example_spec();
        v["resources"]["swapMb"] = serde_json::json!(1024);
        assert!(serde_json::from_value::<WorkloadSpec>(v).is_err());
    }

    /// Whatever a request leaves out of `stop`, `restart` or a port, the whole object or some of
    /// its fields, is the same default; and a field it doesn't know is still refused there.
    #[test]
    fn what_a_spec_leaves_out_takes_the_one_default() {
        use serde_json::json;
        let with = |key: &str, value: Option<serde_json::Value>| {
            let mut v = example_spec();
            match value {
                Some(value) => v[key] = value,
                None => _ = v.as_object_mut().unwrap().remove(key),
            }
            serde_json::from_value::<WorkloadSpec>(v)
        };
        let stop = |signal, timeout_seconds| StopSpec { signal, timeout_seconds };
        for (value, expected) in [
            (None, stop(StopSignal::SIGTERM, 90)),
            (Some(json!({})), stop(StopSignal::SIGTERM, 90)),
            (Some(json!({ "signal": "SIGINT" })), stop(StopSignal::SIGINT, 90)),
            (Some(json!({ "timeoutSeconds": 30 })), stop(StopSignal::SIGTERM, 30)),
        ] {
            assert_eq!(with("stop", value.clone()).expect("parses").stop, expected, "stop: {value:?}");
        }
        let restart = |policy, max_retries| RestartSpec { policy, max_retries };
        for (value, expected) in [
            (None, restart(RestartPolicy::OnFailure, 3)),
            (Some(json!({})), restart(RestartPolicy::OnFailure, 3)),
            (Some(json!({ "policy": "no" })), restart(RestartPolicy::No, 3)),
            (Some(json!({ "maxRetries": 0 })), restart(RestartPolicy::OnFailure, 0)),
        ] {
            assert_eq!(with("restart", value.clone()).expect("parses").restart, expected, "restart: {value:?}");
        }
        let port = json!([{ "name": "http", "containerPort": crate::examples::HTTP_PORT, "audience": ["edge"] }]);
        assert_eq!(with("ports", Some(port)).expect("parses").ports[0].protocol, Proto::Tcp);
        assert!(with("stop", Some(json!({ "signal": "SIGTERM", "graceSeconds": 5 }))).is_err());
        assert!(with("restart", Some(json!({ "backoffSeconds": 5 }))).is_err());
    }

    #[test]
    fn a_secret_never_shows_in_debug_output_or_the_record() {
        let s = spec(example_spec());
        assert!(!format!("{s:?}").contains("hunter2"));
        let record = serde_json::to_string(&s.record()).unwrap();
        assert!(!record.contains("hunter2"));
        assert!(record.contains("APP_TOKEN"), "the name is kept");
    }

    #[test]
    fn the_digest_is_stable_and_ignores_secret_values() {
        let a = spec(example_spec()).record().digest();
        let mut v = example_spec();
        v["secrets"]["APP_TOKEN"] = "another".into();
        assert_eq!(a, spec(v).record().digest(), "secret values are compared on the container, not here");
        let mut v = example_spec();
        v["env"]["VERSION"] = "1.21.9".into();
        assert_ne!(a, spec(v).record().digest());
        assert!(a.starts_with("sha256:") && a.len() == 7 + 64);
        let mut v = example_spec();
        v["resumesOn"] = "1.21.8".into();
        assert_eq!(a, spec(v).record().digest(), "resumesOn is not part of what the spec is");
    }

    #[test]
    fn a_kept_spec_is_whole_again_only_with_every_secret_it_names() {
        let s = spec(example_spec());
        let secrets = BTreeMap::from([("APP_TOKEN".to_owned(), "hunter2hunter2".to_owned())]);
        let again = s.record().with_secrets(&secrets, Some("1.21.8".into())).expect("every secret is there");
        assert_eq!(again.record(), s.record());
        assert_eq!(again.secrets["APP_TOKEN"].expose(), "hunter2hunter2");
        assert_eq!(again.resumes_on.as_deref(), Some("1.21.8"));
        assert!(s.record().with_secrets(&BTreeMap::new(), None).is_none());
    }

    /// Nodes already running these specs stored these digests, in records and container labels.
    /// A build that hashes the same spec differently recreates every container on the next PUT.
    #[test]
    fn the_digest_of_a_spec_is_the_same_in_every_build() {
        for (fixture, expected) in [
            (
                include_str!("../../tests/fixtures/workload-full.json"),
                "sha256:254b6f6c3ff0511614e5d07d04385cfb70f94dd5f05864f59b18f7c3228136a2",
            ),
            (
                include_str!("../../tests/fixtures/workload-minimal.json"),
                "sha256:431a570aeec3f8f524675d78c950fe9d389976be51319d54a8b131aad0a1415b",
            ),
        ] {
            let record: serde_json::Value = serde_json::from_str(fixture).unwrap();
            let spec: SpecRecord = serde_json::from_value(record["spec"].clone()).unwrap();
            assert_eq!(spec.digest(), expected);
        }
    }
}

/// Hostname rules are enforced by the host TCP gateway; deny takes precedence over allow.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EgressPolicy {
    pub(crate) allow: Vec<String>,
    #[serde(default)]
    pub(crate) deny: Vec<String>,
}
