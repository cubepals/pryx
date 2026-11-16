// SPDX-License-Identifier: FSL-1.1-ALv2
//! Agent launch and tree signals. PID start ticks prevent a stale runtime journal from acting
//! on a reused PID, and pidfds bind each signal to the observed process generation.
use super::journal::error;
use super::observe::{identity, matching_pid, tree};
use super::*;
use std::{
    fs,
    os::unix::process::CommandExt,
    process::{Command, Stdio},
};

// Linux setpriority(2) defines the lowest-priority nice value as 19.
const MAX_NICE: i64 = 19;
// The process tier maps this many requested CPU-share weights to one nice step.
const SHARES_PER_NICE_STEP: i64 = 1024;

fn temporary_directory(spec: &ContainerSpec, owner: (u32, u32)) -> Result<PathBuf, RuntimeError> {
    let tmp = spec.data_dir.join(".tmp");
    // DataDir walks each component using openat with O_DIRECTORY and O_NOFOLLOW.
    let data = pryx_agent::paths::DataDir::open(&spec.data_dir).map_err(error)?;
    data.mkdir(".tmp").map_err(error)?;
    let temporary = data.directory_file(".tmp").map_err(error)?;
    rustix::fs::fchown(
        &temporary,
        Some(rustix::process::Uid::from_raw(owner.0)),
        Some(rustix::process::Gid::from_raw(owner.1)),
    )
    .map_err(error)?;
    Ok(tmp)
}

/// Binds the supervisor's listeners here and hands them to it across exec, named in its
/// environment by address and descriptor. Each stays close-on-exec in this daemon, so no other
/// child ever holds one; only the launched child's copy is opened for its exec.
fn pass_listeners(
    command: &mut Command,
    bindings: &BTreeMap<u16, Vec<std::net::IpAddr>>,
) -> Result<Vec<std::os::fd::OwnedFd>, RuntimeError> {
    use std::os::fd::{AsRawFd, OwnedFd};
    let mut held: Vec<OwnedFd> = Vec::new();
    let mut named = Vec::new();
    for (port, ips) in bindings {
        for ip in ips {
            let listener = std::net::TcpListener::bind((*ip, *port)).map_err(error)?;
            let fd = OwnedFd::from(listener);
            named.push(pryx_agent::process_listeners::Inherited {
                address: std::net::SocketAddr::new(*ip, *port),
                fd: fd.as_raw_fd(),
            });
            held.push(fd);
        }
    }
    command.env(pryx_agent::process_listeners::LISTENERS_ENV, serde_json::to_string(&named).map_err(error)?);
    let raw: Vec<i32> = held.iter().map(AsRawFd::as_raw_fd).collect();
    open_across_exec(command, raw);
    Ok(held)
}

#[expect(unsafe_code, reason = "Clearing close-on-exec in the forked child before its exec")]
fn open_across_exec(command: &mut Command, descriptors: Vec<i32>) {
    // SAFETY: the closure only calls fcntl, which is async-signal-safe, on descriptors this
    // daemon owns until the spawn returns; it allocates nothing after the fork.
    unsafe {
        command.pre_exec(move || {
            for fd in &descriptors {
                let fd = std::os::fd::BorrowedFd::borrow_raw(*fd);
                rustix::io::fcntl_setfd(fd, rustix::io::FdFlags::empty())?;
            }
            Ok(())
        });
    }
}

impl ProcessRuntime {
    /// Traverse-only ancestors permit the selected UID to reach its data; private journal,
    /// credentials and workload records retain their own 0700/0600 permissions.
    pub(super) fn traversable(&self, data_dir: &std::path::Path) -> Result<(), RuntimeError> {
        use std::os::unix::fs::PermissionsExt;
        for parent in data_dir.ancestors().skip(1) {
            fs::set_permissions(parent, fs::Permissions::from_mode(0o711)).map_err(error)?;
            if Some(parent) == self.state.root.parent() {
                break;
            }
        }
        Ok(())
    }
    /// Detaches a sandbox's kernel network policy once its processes are gone.
    pub(super) fn release_network(&self, name: &str) -> Result<(), RuntimeError> {
        self.network.as_ref().map_or(Ok(()), |network| network.release(name))
    }
    pub(super) fn running(&self, name: &str) -> Result<bool, RuntimeError> {
        Ok(self
            .observe_entry(name)?
            .is_some_and(|info| matches!(info.status, ContainerStatus::Running | ContainerStatus::Paused)))
    }
    pub(super) fn signal_tree(&self, name: &str, signal: rustix::process::Signal) -> Result<(), RuntimeError> {
        let entry = self.entry(name)?;
        let Some(root) = matching_pid(entry.pid, entry.ticks) else { return Ok(()) };
        // A namespaced root is the helper, which forwards no signal and would die of one: only a
        // stop or continue reaches it, and the supervisor below it gets everything else.
        let skip_root = entry.namespaced.is_some()
            && !matches!(signal, rustix::process::Signal::STOP | rustix::process::Signal::CONT);
        // Freeze the supervisor first, then repeatedly include descendants that raced the scan.
        for _ in 0..3 {
            let mut targets = tree(root);
            targets.retain(|(pid, _)| !skip_root || *pid != root);
            targets.sort_by_key(|(pid, _)| *pid != root);
            for (pid, ticks) in targets {
                let Some(pid) = rustix::process::Pid::from_raw(pid as i32) else { continue };
                if let Ok(fd) = rustix::process::pidfd_open(pid, rustix::process::PidfdFlags::empty())
                    && identity(pid.as_raw_nonzero().get() as u32) == Some(ticks)
                {
                    let _ = rustix::process::pidfd_send_signal(fd, signal);
                }
            }
            if signal != rustix::process::Signal::STOP {
                break;
            }
        }
        Ok(())
    }
    /// The namespace helper this agent executable starts through, probed once, or why not.
    pub(super) fn namespace_helper(&self, binary: &std::path::Path) -> Result<PathBuf, String> {
        if let Some(known) = self.state.namespaces.lock().unwrap().get(binary) {
            return known.clone();
        }
        let owner = (self.policy.uid_first, self.policy.uid_first);
        let found = super::namespaces::proved(binary, owner);
        if let Err(reason) = &found {
            tracing::info!(reason, "process sandboxes start without namespaces");
        }
        self.state.namespaces.lock().unwrap().entry(binary.to_owned()).or_insert(found).clone()
    }
    /// The supervisor's own executable, or the namespace helper starting it in its namespaces,
    /// and whether it is namespaced and has a network of its own.
    fn supervisor_program(
        &self,
        entry: &Entry,
        tools: Option<&pryx_agent::process_tool_policy::Policy>,
    ) -> Result<(Command, bool, bool), RuntimeError> {
        let spec = &entry.spec;
        let agent = spec.agent.as_ref().ok_or_else(|| error("Missing process supervisor"))?;
        let Some(mode) = entry.namespaced else { return Ok((Command::new(&agent.binary), false, false)) };
        let helper = self.namespace_helper(&agent.binary).map_err(error)?;
        let mut kept = vec![(agent.binary.as_path(), false)];
        if let Some(tools) = tools.filter(|tools| tools.unavailable.is_none()) {
            kept.push(pryx_agent::process_namespaces::tool_directory(tools));
        }
        let mut command = Command::new(helper);
        command.args(super::namespaces::arguments(&spec.data_dir, mode.private_network, &kept)).arg(&agent.binary);
        Ok((command, true, mode.private_network))
    }
    fn launch_command(
        &self,
        entry: &Entry,
        tools: Option<pryx_agent::process_tool_policy::Policy>,
        require_scope: bool,
    ) -> Result<(Command, Vec<std::os::fd::OwnedFd>), RuntimeError> {
        let spec = &entry.spec;
        let agent = spec.agent.as_ref().ok_or_else(|| error("Missing process supervisor"))?;
        let owner = self.owner(&agent.sandbox)?;
        self.traversable(&spec.data_dir)?;
        let binding = |logical| {
            spec.ports
                .iter()
                .find(|p| p.container_port == logical && p.protocol == crate::protocol::Proto::Tcp)
                .ok_or_else(|| error("Missing agent port binding"))
        };
        let port = |logical| binding(logical).map(|p| p.host_port);
        let bindings = std::iter::once(agent.agent_port)
            .chain(agent.spare_ports.iter().copied())
            .map(|logical| {
                let bound = binding(logical)?;
                if bound.host_ips.is_empty() || bound.host_ips.iter().any(std::net::IpAddr::is_unspecified) {
                    return Err(error("Process listeners require explicit host addresses"));
                }
                Ok((bound.host_port, bound.host_ips.clone()))
            })
            .collect::<Result<BTreeMap<_, _>, RuntimeError>>()?;
        let spares =
            agent.spare_ports.iter().map(|p| port(*p).map(|p| p.to_string())).collect::<Result<Vec<_>, _>>()?;
        let (mut command, namespaced, private_network) = self.supervisor_program(entry, tools.as_ref())?;
        command.args(["--host-bindings", &serde_json::to_string(&bindings).map_err(error)?]);
        if let Some(origin) = &agent.public_origin {
            command.args(["--public-origin", origin]);
        }
        command.args([
            "--sandbox",
            &agent.sandbox,
            "--epoch",
            &agent.epoch.to_string(),
            "--data",
            &spec.data_dir.to_string_lossy(),
            "--domain",
            &agent.domain,
            "--port",
            &port(agent.agent_port)?.to_string(),
            "--secret-env",
            &agent.secret_env,
            "--entrypoint-cwd",
            spec.workdir.as_deref().unwrap_or(&spec.data_dir.to_string_lossy()),
        ]);
        if !spares.is_empty() {
            command.args(["--spare-ports", &spares.join(",")]);
        }
        if agent.keep_alive || spec.entrypoint.is_none() {
            command.arg("--keep-alive");
        }
        if !agent.keep_alive
            && let Some(argv) = &spec.entrypoint
        {
            command.arg("--").args(argv);
        }
        let tmp = temporary_directory(spec, owner)?;
        let mut policy = serde_json::json!({"data":spec.data_dir, "memory_bytes":spec.memory_bytes, "address_space_bytes":self.policy.address_space_mb.saturating_mul(1024*1024), "open_files":self.policy.open_files, "processes":spec.pids_limit, "cpu_seconds":self.policy.cpu_seconds, "nice":(MAX_NICE-(spec.cpu_shares/SHARES_PER_NICE_STEP).min(MAX_NICE)) as i32, "network_off":spec.internal_network, "private_tmp":namespaced.then_some(true), "require_ipc_scope":require_scope.then_some(true)});
        // An unset flag is no field at all, as older agents' policies have it.
        policy.as_object_mut().expect("a JSON object").retain(|_, value| !value.is_null());
        if let Some(tools) = tools {
            policy["tools"] = serde_json::to_value(tools).map_err(error)?;
        }
        command
            .env_clear()
            .env("PATH", "/usr/local/bin:/usr/bin:/bin")
            .env("HOME", &spec.data_dir)
            .env("TMPDIR", &tmp)
            .env("TERM", "xterm-256color")
            .envs(spec.env.iter().cloned());
        command.env("PRYX_PROCESS_POLICY", serde_json::to_string(&policy).map_err(error)?);
        if spec.internal_network {
            command.env("PRYX_PROCESS_NETWORK_OFF", "1");
        }
        let listeners = if private_network { pass_listeners(&mut command, &bindings)? } else { Vec::new() };
        command
            .current_dir(&spec.data_dir)
            .uid(owner.0)
            .gid(owner.1)
            .process_group(0)
            .stdin(Stdio::null())
            .stdout(Stdio::null());
        let diagnostic = self.state.root.join(format!("{}.stderr", entry.id));
        use std::os::unix::fs::OpenOptionsExt;
        let diagnostic = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(diagnostic)
            .map_err(error)?;
        command.stderr(diagnostic);
        Ok((command, listeners))
    }
    pub(super) async fn launch(&self, name: &str) -> Result<(), RuntimeError> {
        let _launch = self.state.launch.lock().await;
        let mut entry = self.entry(name)?;
        if entry.spec.nano_cpus.is_some() {
            return Err(error(crate::protocol::CPU_LIMIT_PROBLEM));
        }
        if self.running(name)? {
            return Ok(());
        }
        let (tools, required) = self.native_policy(&entry).await?;
        entry.native_browser =
            Some(tools.as_ref().is_some_and(|tools| tools.unavailable.is_none() && tools.browser_on));
        let binary = entry.spec.agent.as_ref().map(|agent| agent.binary.clone());
        let proved = match binary {
            Some(binary) => {
                let own = self.clone();
                tokio::task::spawn_blocking(move || own.namespace_helper(&binary).is_ok()).await.unwrap_or(false)
            }
            None => false,
        };
        entry.namespaced =
            proved.then_some(super::namespaces::Namespaced { private_network: entry.spec.internal_network });
        let (mut command, listeners) = self.launch_command(&entry, tools, required)?;
        crate::bpf::host::drop_ambient_on_exec(&mut command);
        // Every socket the supervisor or its commands open is its cgroup's, so the programs
        // are attached before it starts and it joins the cgroup before it runs. A namespaced
        // supervisor's helper joins it the same way, and everything below it inherits it.
        if let Some(network) = &self.network {
            let (record, cgroup) = network.prepare(&entry.spec).await?;
            entry.network = Some(record);
            if let Err(problem) = crate::bpf::host::join_on_exec(&mut command, &cgroup) {
                network.release(&entry.spec.name)?;
                return Err(error(problem));
            }
        }
        let child = match command.spawn() {
            Ok(child) => child,
            Err(problem) => {
                self.release_network(&entry.spec.name)?;
                return Err(error(problem));
            }
        };
        drop(listeners);
        entry.pid = Some(child.id());
        entry.ticks = identity(child.id());
        entry.started = Some(format_time(crate::clock::now()));
        entry.cleaned = false;
        entry.finished = None;
        self.state.children.lock().unwrap().insert(entry.spec.name.clone(), child);
        self.save(entry)?;
        // Health is a safe read; startup failures never cause a duplicate launch.
        for _ in 0..500 {
            if !self.running(name)? {
                let exit = self.entry(name)?;
                let code = exit.finished.map(|_| exit.code.to_string()).unwrap_or_else(|| "unconfirmed".into());
                return Err(error(format!(
                    "Process supervisor exited during startup (exit {code}); inspect its private diagnostic"
                )));
            }
            if self.agent_health(name).await.is_ok() {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        Err(RuntimeError::Timeout("Process supervisor did not become healthy".into()))
    }
    pub(super) fn observe_entry(&self, name: &str) -> Result<Option<ContainerInfo>, RuntimeError> {
        // Observation and journal removal share the same turn; a late observation cannot
        // recreate removed compute or replace a newer exit status.
        let mut entries = self.state.entries.lock().unwrap();
        let mut entry = match Self::entry_from(&entries, name) {
            Ok(entry) => entry,
            Err(RuntimeError::NotFound(_)) => return Ok(None),
            Err(e) => return Err(e),
        };
        let exited = self
            .state
            .children
            .lock()
            .unwrap()
            .get_mut(&entry.spec.name)
            .and_then(|child| child.try_wait().ok().flatten());
        if let Some(status) = exited {
            use std::os::unix::process::ExitStatusExt;
            entry.code = status.code().map_or_else(|| exit_by(status.signal().unwrap_or(libc::SIGKILL)), i64::from);
            entry.finished = Some(format_time(crate::clock::now()));
            self.state.children.lock().unwrap().remove(&entry.spec.name);
            self.save_in(&mut entries, entry.clone())?;
        }
        let alive = matching_pid(entry.pid, entry.ticks).is_some();
        if !alive && !entry.cleaned {
            if let Some(agent) = &entry.spec.agent {
                self.retire_orphans(&agent.sandbox)?;
            }
            self.release_network(&entry.spec.name)?;
            entry.cleaned = true;
            self.save_in(&mut entries, entry.clone())?;
        }
        let paused = alive
            && entry
                .pid
                .and_then(|pid| fs::read_to_string(format!("/proc/{pid}/stat")).ok())
                .and_then(|s| s.rsplit_once(") ").map(|(_, s)| s.starts_with('T')))
                .unwrap_or(false);
        let mut labels = entry.spec.labels;
        labels.insert(
            crate::labels::LABEL_BROWSER_SANDBOX.into(),
            (alive && entry.native_browser == Some(true)).to_string(),
        );
        let attached = self.network.as_ref().is_some_and(|network| network.verified(&entry.spec.name));
        labels.insert(crate::labels::LABEL_NETWORK_POLICY.into(), (alive && attached).to_string());
        labels.insert(crate::labels::LABEL_PRIVATE_PIDS.into(), entry.namespaced.is_some().to_string());
        labels.insert(
            crate::labels::LABEL_PRIVATE_NETWORK.into(),
            entry.namespaced.is_some_and(|mode| mode.private_network).to_string(),
        );
        Ok(Some(ContainerInfo {
            agent_state: None,
            memory_enforced: None,
            cpu_enforced: None,
            cgroup_parent: None,
            id: entry.id,
            name: entry.spec.name,
            labels,
            env: entry.spec.env.into_iter().collect(),
            status: if paused {
                ContainerStatus::Paused
            } else if alive {
                ContainerStatus::Running
            } else if entry.pid.is_none() {
                ContainerStatus::Created
            } else {
                ContainerStatus::Exited
            },
            exit_code: entry.code,
            oom_killed: false,
            started_at: parse_time(entry.started.as_deref()),
            finished_at: parse_time(entry.finished.as_deref()),
            restart_count: 0,
            health: None,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};

    #[test]
    fn launch_refuses_a_temporary_directory_symlink_without_touching_its_target() {
        let root = tempfile::tempdir().unwrap();
        let state = root.path().join("state");
        let runtime = ProcessRuntime::new(&state, ProcessConfig::default()).unwrap();
        let data = state.join("data");
        fs::create_dir(&data).unwrap();
        let outside = root.path().join("outside");
        fs::create_dir(&outside).unwrap();
        fs::set_permissions(&outside, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(outside.join("marker"), b"untouched").unwrap();
        symlink(&outside, data.join(".tmp")).unwrap();
        let before = fs::metadata(&outside).unwrap();
        let spec: ContainerSpec = serde_json::from_value(serde_json::json!({
            "name": "tmp-link", "image": "", "env": [], "labels": {},
            "agent": {"binary": "/nonexistent-agent", "sandbox": "tmp-link", "epoch": 1,
                "secret_env": "TEST_AGENT_SECRET", "domain": "pryx.test",
                "agent_port": pryx_agent::contract::AGENT_PORT, "spare_ports": [], "keep_alive": true},
            "user": "", "memory_bytes": 0, "cpu_shares": 0, "pids_limit": 0,
            "read_only_rootfs": true, "tmp_size_mb": 0, "data_dir": data,
            "mount_path": "/data", "ports": [{"container_port": pryx_agent::contract::AGENT_PORT,
                "host_port": pryx_agent::contract::AGENT_PORT, "host_ips": ["127.0.0.1"], "protocol": "tcp"}],
            "stop_signal": "SIGTERM", "stop_timeout_secs": 0, "network": "", "internal_network": false,
            "log_max_size_mb": 0, "log_max_files": 0, "oom_score_adj": 0
        }))
        .unwrap();
        runtime.create_entry(&spec).unwrap();
        let result =
            runtime.launch_command(&runtime.entry(&spec.name).unwrap(), None, false).map(|(command, _)| command);
        let after = fs::metadata(&outside).unwrap();
        assert_eq!(
            (after.uid(), after.gid(), after.mode(), after.ctime(), after.ctime_nsec()),
            (before.uid(), before.gid(), before.mode(), before.ctime(), before.ctime_nsec()),
            "the daemon changed the outside directory"
        );
        assert_eq!(fs::read(outside.join("marker")).unwrap(), b"untouched");
        assert_eq!(fs::read_dir(&outside).unwrap().count(), 1);
        assert!(result.is_err(), "a temporary-directory symlink must prevent launch");
    }

    #[test]
    fn launch_passes_each_host_binding_to_the_agent_and_refuses_wildcards() {
        let root = tempfile::tempdir().unwrap();
        let runtime = ProcessRuntime::new(root.path(), ProcessConfig::default()).unwrap();
        let data = root.path().join("data");
        fs::create_dir(&data).unwrap();
        let logical = pryx_agent::contract::AGENT_PORT;
        let spec: ContainerSpec = serde_json::from_value(serde_json::json!({
            "name": "bound", "image": "", "env": [], "labels": {},
            "agent": {"binary": "/nonexistent-agent", "sandbox": "bound", "epoch": 1,
                "secret_env": "TEST_AGENT_SECRET", "domain": "pryx.test",
                "agent_port": logical, "spare_ports": [logical + 1], "keep_alive": true},
            "user": "", "memory_bytes": 0, "cpu_shares": 0, "pids_limit": 0,
            "read_only_rootfs": true, "tmp_size_mb": 0, "data_dir": data,
            "mount_path": "/data", "ports": [
                {"container_port": logical, "host_port": logical + 100, "host_ips": ["127.0.0.1", "::1"], "protocol": "tcp"},
                {"container_port": logical + 1, "host_port": logical + 101, "host_ips": ["127.0.0.2"], "protocol": "tcp"}],
            "stop_signal": "SIGTERM", "stop_timeout_secs": 0, "network": "", "internal_network": false,
            "log_max_size_mb": 0, "log_max_files": 0, "oom_score_adj": 0
        })).unwrap();
        runtime.create_entry(&spec).unwrap();
        let mut entry = runtime.entry(&spec.name).unwrap();
        let (command, _) = runtime.launch_command(&entry, None, false).unwrap();
        let arguments = command.get_args().map(|a| a.to_str().unwrap()).collect::<Vec<_>>();
        let option = arguments.iter().position(|a| *a == "--host-bindings").unwrap();
        let bindings: BTreeMap<u16, Vec<std::net::IpAddr>> = serde_json::from_str(arguments[option + 1]).unwrap();
        assert_eq!(
            bindings[&(logical + 100)],
            vec!["127.0.0.1".parse::<std::net::IpAddr>().unwrap(), "::1".parse().unwrap()]
        );
        assert_eq!(bindings[&(logical + 101)], vec!["127.0.0.2".parse::<std::net::IpAddr>().unwrap()]);
        entry.spec.ports[0].host_ips.clear();
        assert!(runtime.launch_command(&entry, None, false).is_err());
        entry.spec.ports[0].host_ips = vec![std::net::Ipv4Addr::UNSPECIFIED.into()];
        assert!(runtime.launch_command(&entry, None, false).is_err());
    }
}
