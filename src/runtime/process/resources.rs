// SPDX-License-Identifier: FSL-1.1-ALv2
//! Each process sandbox's memory, CPU and process count, enforced by the kernel: a cgroup of its
//! own in this daemon's delegated subtree (`crate::cgroup`), its limits written and read back
//! before its supervisor runs, frozen whole for sleep and killed whole once it stops. Where the
//! host delegates no controllers this is absent: the tier reports memory-limit and cpu-limit off
//! with the reason, memory stays the agent's sampled budget, and a CPU ceiling is refused naming
//! cpu-limit. Not the network programs that may share the cgroup (`network.rs`), nor the launch
//! itself (`power.rs`).
use super::journal::error;
use super::*;
use crate::capability::{Capability, State as Proved};
use crate::cgroup::{
    delegation::{self, CPU, Delegated, MEMORY, PIDS},
    limits::{self, Limits},
};
use crate::protocol::Guarantee;
use std::{fs, path::Path};

/// The host probe's name for kernel-enforced process sandbox limits.
pub(crate) const CAPABILITY: &str = "process-limits";

/// What a sandbox started with, kept in its journal: its cgroup, and which of its limits the
/// kernel enforces there.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Record {
    pub(super) cgroup: PathBuf,
    pub(super) memory: bool,
    pub(super) cpu: bool,
    pub(super) pids: bool,
}

/// This daemon's subtree, with the controllers it gives each sandbox.
pub(super) struct Resources {
    delegated: Delegated,
}

impl Resources {
    pub(super) fn has(&self, controller: &str) -> bool {
        self.delegated.has(controller)
    }

    /// A sandbox's cgroup, the one its network policy's programs attach to as well.
    fn cgroup(&self, key: &str) -> PathBuf {
        self.delegated.root.join(format!("pryx-{key}"))
    }

    /// What the kernel enforces for `spec` here: its memory with no swap beyond it, a CPU ceiling
    /// if it named one, and its process count.
    pub(super) fn limits(&self, spec: &ContainerSpec) -> Limits {
        let positive = |value: i64| u64::try_from(value).ok().filter(|value| *value > 0);
        Limits {
            memory_bytes: self.has(MEMORY).then(|| positive(spec.memory_bytes)).flatten(),
            cpu_quota_micros: self.has(CPU).then(|| spec.nano_cpus.and_then(limits::cpu_quota)),
            pids: self.has(PIDS).then(|| positive(spec.pids_limit)).flatten(),
        }
    }

    /// Writes `spec`'s limits into `cgroup` and reads them back: a sandbox runs with exactly what
    /// it asked for, or not at all.
    fn apply(&self, cgroup: &Path, spec: &ContainerSpec) -> Result<Record, RuntimeError> {
        let wanted = self.limits(spec);
        if spec.nano_cpus.is_some() && wanted.cpu_quota_micros.flatten().is_none() {
            return Err(error(cpu_problem(None)));
        }
        limits::write(cgroup, &wanted).map_err(|problem| error(format!("Process limits: {problem}")))?;
        let found = limits::read(cgroup).map_err(|problem| error(format!("Process limits: {problem}")))?;
        let kept = |asked: Option<u64>, found: Option<u64>| asked.is_none() || asked == found;
        if !kept(wanted.memory_bytes, found.memory_bytes)
            || !kept(wanted.pids, found.pids)
            || wanted.cpu_quota_micros.is_some_and(|quota| Some(quota) != found.cpu_quota_micros)
        {
            return Err(error(format!(
                "Process limits: {} reads {found:?} after {wanted:?} was written",
                cgroup.display()
            )));
        }
        Ok(Record {
            cgroup: cgroup.to_path_buf(),
            memory: wanted.memory_bytes.is_some(),
            cpu: wanted.cpu_quota_micros.is_some(),
            pids: wanted.pids.is_some(),
        })
    }
}

/// Why a CPU ceiling can't be enforced on this host's process tier, naming the guarantee.
fn cpu_problem(capability: Option<&Capability>) -> String {
    let (evidence, action) = match capability.filter(|capability| capability.state != Proved::On) {
        Some(capability) => (capability.evidence.as_str(), capability.action.as_str()),
        None => ("this daemon's cgroup gives its sandboxes no cpu controller", delegation::DELEGATE_ACTION),
    };
    format!("{} is unavailable on this host's process tier: {evidence}; {action}", Guarantee::CpuLimit.feature())
}

/// What the host document says of `found`: on where each sandbox gets both a memory and a CPU
/// limit, else off with what is missing.
fn capability(found: &Result<Delegated, crate::cgroup::Unavailable>) -> Capability {
    match found {
        Ok(delegated) if delegated.has(MEMORY) && delegated.has(CPU) => Capability::on(format!(
            "each sandbox gets a cgroup below {} with {}",
            delegated.root.display(),
            delegated.controllers.join(", ")
        )),
        Ok(delegated) => Capability::new(
            Proved::Off,
            format!(
                "cgroup {} gives its sandboxes only {}",
                delegated.root.display(),
                delegated.controllers.join(", ")
            ),
            delegation::DELEGATE_ACTION,
        ),
        Err(problem) => Capability::new(Proved::Off, problem.evidence.clone(), problem.action),
    }
}

/// The same report for a caller that isn't the daemon (`pryxd probe`): what its cgroup could give,
/// with nothing moved or turned on.
pub(crate) fn inspected() -> Capability {
    capability(&delegation::inspect())
}

/// Kills whatever is left in a sandbox's cgroup, waits for it to empty and removes it. A cgroup
/// already gone is released.
pub(super) fn release(cgroup: &Path) -> Result<(), RuntimeError> {
    match fs::symlink_metadata(cgroup) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => return Err(error(format!("Process limits: {} is not a cgroup", cgroup.display()))),
        Err(problem) if problem.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(problem) => return Err(error(problem)),
    }
    let failed = |problem: std::io::Error| error(format!("Process limits: releasing {}: {problem}", cgroup.display()));
    let usage = limits::usage(cgroup);
    if usage.oom_kills.is_some_and(|kills| kills > 0) {
        tracing::info!(cgroup = %cgroup.display(), oom_kills = usage.oom_kills, "process sandbox's commands passed its memory.max");
    }
    crate::cgroup::fence_cgroup(cgroup).map_err(failed)?;
    crate::cgroup::wait_empty(cgroup).map_err(failed)?;
    crate::cgroup::remove_cgroup(cgroup).map_err(failed)
}

pub(super) fn pending_limits(entry: &Entry) -> Option<&Record> {
    entry.limits.as_ref().filter(|_| entry.network_launch_pending && entry.network.is_none())
}

impl ProcessRuntime {
    /// Turns kernel-enforced limits on where this daemon's cgroup can give its children memory
    /// and cpu (and pids). Runs before anything else this daemon starts, since the daemon may move
    /// into its leaf only while it is its cgroup's one process. A supervisor that outlived the last
    /// daemon keeps what its record says only while it is still in that cgroup.
    pub(crate) fn with_resources(mut self) -> Self {
        let found = delegation::delegate();
        let capability = capability(&found);
        tracing::info!(on = capability.state == Proved::On, evidence = %capability.evidence, "process limits");
        self.resources_capability = Some(capability);
        self.resources = found.ok().map(|delegated| Arc::new(Resources { delegated }));
        for name in self.names() {
            let Ok(mut entry) = self.entry(&name) else { continue };
            let Some(record) = &entry.limits else { continue };
            let Some(pid) = super::observe::matching_pid(entry.pid, entry.ticks) else {
                if pending_limits(&entry).is_some()
                    && let Err(problem) = self.observe_entry(&name)
                {
                    tracing::warn!(sandbox = %name, %problem, "incomplete process launch still awaits cgroup retirement");
                }
                continue;
            };
            if crate::cgroup::members(&record.cgroup).is_ok_and(|members| members.contains(&pid)) {
                continue;
            }
            tracing::warn!(sandbox = %entry.spec.name, "process sandbox is no longer in its limits' cgroup");
            entry.limits = None;
            if let Err(problem) = self.save(entry) {
                tracing::warn!(sandbox = %name, %problem, "process sandbox's lost limits were not saved");
            }
        }
        self
    }

    /// Retire the cgroup named by a limits-only intent without a live PID checkpoint. The
    /// caller saves the cleared intent after retirement, so observation can retry on failure.
    pub(super) fn retire_pending_limits(&self, entry: &mut Entry) -> Result<bool, RuntimeError> {
        let Some(record) = pending_limits(entry) else { return Ok(false) };
        // A proven new boot already ended this generation. Otherwise its owned cgroup is
        // the retirement proof, including if the child was still in pre-exec.
        if self.legacy_pending(entry) {
            release(&record.cgroup)?;
        }
        entry.limits = None;
        entry.network_launch_pending = false;
        entry.network_clean = true;
        entry.cleaned = true;
        Ok(true)
    }

    /// The process tier's kernel-enforced limits as the host document reports them.
    pub(super) fn limits_capability(&self) -> Option<Capability> {
        self.resources_capability.clone()
    }

    /// The agent's sampled memory budget for `spec`. Where memory.max holds each sandbox it is set
    /// past reach: resident pages summed, shared ones more than once, would only kill a sandbox
    /// the kernel lets run.
    pub(super) fn sampled_budget(&self, spec: &ContainerSpec) -> u64 {
        let budget = u64::try_from(spec.memory_bytes).unwrap_or_default();
        let kernel = self.resources.as_ref().is_some_and(|resources| resources.has(MEMORY));
        if kernel && budget > 0 { u64::MAX } else { budget }
    }

    /// Why a CPU ceiling can't be enforced here, or `None` where each sandbox's cgroup has cpu.
    pub(super) fn cpu_problem(&self) -> Option<String> {
        match &self.resources {
            Some(resources) if resources.has(CPU) => None,
            _ => Some(cpu_problem(self.resources_capability.as_ref())),
        }
    }

    /// Before a supervisor starts: its cgroup, made here unless its network policy made it, with
    /// its limits written. On failure nothing of it is left and the launch is refused.
    pub(super) fn prepare_limits(&self, entry: &Entry) -> Result<Option<Record>, RuntimeError> {
        let Some(resources) = &self.resources else { return Ok(None) };
        let cgroup = resources.cgroup(&super::network::key(&self.state.root, &entry.spec.name));
        let prepared = (|| {
            if self.network.is_none() {
                release(&cgroup)?;
                crate::cgroup::create_cgroup(&cgroup).map_err(|problem| error(format!("Process limits: {problem}")))?;
            }
            resources.apply(&cgroup, &entry.spec)
        })();
        if prepared.is_err() {
            self.release_network(entry)?;
            release(&cgroup)?;
        }
        prepared.map(Some)
    }

    /// Sleep: the whole cgroup frozen at once where the sandbox has one, so nothing forks past
    /// it; else every process of its tree stopped. Either way its memory stays where it is.
    pub(super) async fn freeze_sandbox(&self, name: &str) -> Result<(), RuntimeError> {
        let entry = self.entry(name)?;
        match &entry.limits {
            Some(record) if super::observe::matching_pid(entry.pid, entry.ticks).is_some() => {
                let cgroup = record.cgroup.clone();
                tokio::task::spawn_blocking(move || limits::freeze(&cgroup, true))
                    .await
                    .map_err(error)?
                    .map_err(|problem| error(format!("Process limits: {problem}")))
            }
            _ => self.freeze_tree(name).await.map(drop),
        }
    }

    /// Wake: its cgroup thawed and every process of its tree continued. A cgroup its network
    /// policy fenced stays frozen: it may only be started again with new programs.
    pub(super) fn thaw_sandbox(&self, name: &str) -> Result<(), RuntimeError> {
        let entry = self.entry(name)?;
        let fenced =
            entry.network.is_some() && !self.network.as_ref().is_some_and(|network| network.verified(&entry.spec.name));
        if let Some(record) = &entry.limits
            && !fenced
            && super::observe::matching_pid(entry.pid, entry.ticks).is_some()
        {
            limits::freeze(&record.cgroup, false).map_err(|problem| error(format!("Process limits: {problem}")))?;
        }
        self.signal_tree(name, rustix::process::Signal::CONT)
    }

    /// The kernel's cgroup counters, with sampled RSS when a memory count is unavailable.
    pub(super) fn resource_stats(&self, entry: &Entry, pid: u32) -> RawStats {
        let usage = entry.limits.as_ref().map(|record| limits::usage(&record.cgroup)).unwrap_or_default();
        RawStats {
            cpu_total_ns: usage
                .cpu_micros
                .and_then(|micros| u64::try_from(std::time::Duration::from_micros(micros).as_nanos()).ok()),
            cpu_throttled_periods: usage.throttled_periods,
            memory_usage: usage.memory_bytes.or_else(|| observe::resident_tree(pid)),
            memory_inactive_file: usage.memory_bytes.and(usage.inactive_file_bytes),
            memory_limit: u64::try_from(entry.spec.memory_bytes).ok(),
            pids: usage.pids,
            pids_limit: u64::try_from(entry.spec.pids_limit).ok(),
            ..Default::default()
        }
    }
}

/// Starts `command` as a member of `cgroup`, which it joins before it runs anything.
pub(super) fn spawn_in(
    cgroup: &Path,
    command: &mut std::process::Command,
) -> Result<std::process::Child, RuntimeError> {
    crate::cgroup::join_on_exec(command, cgroup).map_err(|problem| error(format!("Process limits: {problem}")))?;
    command.spawn().map_err(error)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resources(controllers: Vec<&'static str>) -> Resources {
        Resources { delegated: Delegated { root: "/sys/fs/cgroup/pryxd.service".into(), controllers } }
    }

    #[test]
    fn a_spec_becomes_the_limits_its_controllers_can_enforce() {
        let root = tempfile::tempdir().unwrap();
        let mut spec = super::super::network::fixture_spec(root.path());
        spec.memory_bytes = 256 * 1024 * 1024;
        spec.pids_limit = 512;
        let all = resources(vec![MEMORY, CPU, PIDS]);
        assert_eq!(
            all.limits(&spec),
            Limits { memory_bytes: Some(268_435_456), cpu_quota_micros: Some(None), pids: Some(512) }
        );
        spec.nano_cpus = Some(250_000_000);
        assert_eq!(all.limits(&spec).cpu_quota_micros, Some(Some(25_000)));
        let memory = resources(vec![MEMORY, PIDS]);
        assert_eq!(memory.limits(&spec).cpu_quota_micros, None);
        assert_eq!(all.cgroup("0123456789abcdef"), Path::new("/sys/fs/cgroup/pryxd.service/pryx-0123456789abcdef"));
    }

    #[test]
    fn limits_are_written_proved_and_recorded() {
        let root = tempfile::tempdir().unwrap();
        let mut spec = super::super::network::fixture_spec(root.path());
        spec.memory_bytes = 128 * 1024 * 1024;
        spec.pids_limit = 64;
        spec.nano_cpus = Some(100_000_000);
        for file in ["memory.max", "cpu.max", "pids.max"] {
            fs::write(root.path().join(file), "max\n").unwrap();
        }
        let record = resources(vec![MEMORY, CPU, PIDS]).apply(root.path(), &spec).unwrap();
        assert_eq!(record, Record { cgroup: root.path().into(), memory: true, cpu: true, pids: true });
        assert_eq!(fs::read_to_string(root.path().join("cpu.max")).unwrap(), "10000 100000");
        // A ceiling the cgroup can't hold is refused before anything runs, naming the guarantee.
        let refused = resources(vec![MEMORY, PIDS]).apply(root.path(), &spec).unwrap_err().to_string();
        assert!(refused.contains("cpu-limit") && refused.contains("no cpu controller"), "{refused}");
    }

    #[test]
    fn the_host_document_names_what_is_missing() {
        let all = Delegated { root: "/sys/fs/cgroup/u".into(), controllers: vec![MEMORY, CPU, PIDS] };
        assert_eq!(capability(&Ok(all)).state, Proved::On);
        let partial = capability(&Ok(Delegated { root: "/sys/fs/cgroup/u".into(), controllers: vec![MEMORY, PIDS] }));
        assert_eq!(partial.state, Proved::Off);
        assert!(partial.evidence.contains("only memory, pids"), "{}", partial.evidence);
        let refused = crate::cgroup::Unavailable { evidence: "cgroup x is not delegated".into(), action: "delegate" };
        let off = capability(&Err(refused));
        assert_eq!(
            (off.state, off.evidence.as_str(), off.action.as_str()),
            (Proved::Off, "cgroup x is not delegated", "delegate")
        );
        assert!(cpu_problem(Some(&off)).starts_with("cpu-limit is unavailable on this host's process tier: cgroup x"));
    }

    #[test]
    fn releasing_a_cgroup_that_is_gone_is_done() {
        release(Path::new("/nonexistent-pryx-cgroup")).unwrap();
    }

    #[tokio::test]
    async fn an_interrupted_limits_only_launch_requires_cgroup_retirement() {
        let root = tempfile::tempdir().unwrap();
        let policy = crate::config::ProcessConfig::default();
        let runtime = ProcessRuntime::new(root.path(), policy.clone()).unwrap();
        let mut spec = super::super::network::fixture_spec(root.path());
        spec.agent = Some(AgentBoot {
            public_origin: None,
            binary: root.path().join("owned-supervisor-fixture"),
            sandbox: spec.name.clone(),
            epoch: 1,
            secret_env: "FIXTURE_ONLY".into(),
            domain: "pryx.test".into(),
            agent_port: pryx_agent::contract::AGENT_PORT,
            spare_ports: Vec::new(),
            keep_alive: true,
        });
        let id = runtime.create_entry(&spec).unwrap();
        let mut entry = runtime.entry(&id).unwrap();
        let cgroup = root.path().join(format!("pryx-{}", super::super::network::key(&runtime.state.root, &spec.name)));
        fs::create_dir(&cgroup).unwrap();
        let frozen = cgroup.join("cgroup.freeze");
        fs::write(&frozen, b"0").unwrap();
        // Ordinary files simulate an owned member and a cgroup.kill operation that fails.
        fs::write(cgroup.join("cgroup.procs"), format!("{}\n", u32::MAX)).unwrap();
        fs::create_dir(cgroup.join("cgroup.kill")).unwrap();
        entry.limits = Some(Record { cgroup: cgroup.clone(), memory: true, cpu: true, pids: false });
        entry.network_boot = Some(journal::UNKNOWN_BOOT.into());
        entry.network_launch_pending = true;
        // The intent is durable, but the daemon lost the PID checkpoint and kept `cleaned`.
        assert!(entry.cleaned && entry.pid.is_none() && entry.network.is_none());
        runtime.save(entry).unwrap();
        drop(runtime);

        let recovered = ProcessRuntime::new(root.path(), policy.clone()).unwrap();
        let observed = recovered.inspect(&id).await;
        assert_eq!(fs::read(&frozen).unwrap(), b"1", "restart must fence the pending limits-only cgroup");
        assert!(observed.is_err(), "an unretired cgroup must refuse observation as a stopped sandbox");
        let pending = recovered.entry(&id).unwrap();
        assert!(pending.network_launch_pending, "retain ownership until retirement is proved");
        assert_eq!(pending.limits.unwrap().cgroup, cgroup);
        recovered.legacy_history().unwrap();
        let pending = recovered.entry(&id).unwrap();
        assert!(recovered.legacy_pending(&pending), "unretired compute must still count during policy adoption");
        assert!(!recovered.legacy_boot_uncertain(), "a recorded cgroup retains a way to prove retirement");

        fs::remove_dir_all(&cgroup).unwrap();
        assert!(matches!(recovered.inspect(&id).await.unwrap().unwrap().status, ContainerStatus::Created));
        drop(recovered);
        let recovered = ProcessRuntime::new(root.path(), policy).unwrap();
        let retired = recovered.entry(&id).unwrap();
        assert!(retired.cleaned && retired.network_clean && !retired.network_launch_pending);
        assert!(retired.limits.is_none());
        assert!(!recovered.legacy_boot_uncertain());
    }

    #[tokio::test]
    async fn stats_keep_sampled_memory_with_cpu_only_delegation() {
        let root = tempfile::tempdir().unwrap();
        let runtime = ProcessRuntime::new(root.path(), crate::config::ProcessConfig::default()).unwrap();
        let spec = super::super::network::fixture_spec(root.path());
        let id = "cpu-only";
        let cgroup = root.path().join("cgroup");
        fs::create_dir(&cgroup).unwrap();
        fs::write(cgroup.join("cpu.stat"), "usage_usec 2000\nnr_throttled 7\n").unwrap();
        fs::write(cgroup.join("memory.stat"), "inactive_file 1024\n").unwrap();

        let mut child = tokio::process::Command::new("sleep").arg("30").kill_on_drop(true).spawn().unwrap();
        let pid = child.id().unwrap();
        let entry = serde_json::from_value(serde_json::json!({
            "id": id, "spec": spec, "pid": pid, "ticks": observe::identity(pid).unwrap(), "code": 0,
            "limits": Record { cgroup: cgroup.clone(), memory: false, cpu: true, pids: false },
        }))
        .unwrap();
        runtime.save(entry).unwrap();

        let stats = runtime.stats(id).await.unwrap().unwrap();
        assert!(stats.memory_usage.is_some(), "sampled RSS must survive partial delegation");
        assert_eq!(stats.memory_inactive_file, None);
        assert_eq!(stats.cpu_total_ns, Some(2_000_000));
        assert_eq!(stats.cpu_throttled_periods, Some(7));

        fs::write(cgroup.join("memory.current"), "4096\n").unwrap();
        let kernel = runtime.stats(id).await.unwrap().unwrap();
        assert_eq!(kernel.memory_usage, Some(4096));
        assert_eq!(kernel.memory_inactive_file, Some(1024));
        child.kill().await.unwrap();
    }
}
