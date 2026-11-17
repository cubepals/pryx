// SPDX-License-Identifier: FSL-1.1-ALv2
//! Bounded sideband evidence for the existing real Docker OOM test. Sampling begins after
//! Start, changes no workload or deadline, and never substitutes for its OOM assertion. Values
//! are first/last observations, not a baseline before execution or a final count after deletion.
use super::oom_fields::container_id;
use super::oom_owner::{Failure, Group, GroupReport, Process, Sample};
use bollard::models::ContainerInspectResponse;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

#[derive(Clone, serde::Serialize)]
struct Report {
    id: Option<String>,
    started_at_unix_ns: Option<i128>,
    host_pid: Option<i32>,
    start_ticks: Option<u64>,
    configured_memory_bytes: Option<i64>,
    pidfd_opened: bool,
    owner_confirmed: bool,
    cgroup: Option<GroupReport>,
    first_observed: Option<Sample>,
    last_observed: Option<Sample>,
    samples: u32,
    stage: &'static str,
    errno: Option<i32>,
}

impl Report {
    fn new() -> Self {
        Self {
            id: None,
            started_at_unix_ns: None,
            host_pid: None,
            start_ticks: None,
            configured_memory_bytes: None,
            pidfd_opened: false,
            owner_confirmed: false,
            cgroup: None,
            first_observed: None,
            last_observed: None,
            samples: 0,
            stage: "pending",
            errno: None,
        }
    }
    fn failed(&mut self, failure: Failure) {
        self.stage = failure.stage;
        self.errno = failure.errno;
    }
}

pub(super) struct Capture {
    report: Arc<Mutex<Report>>,
    stop: CancellationToken,
    task: tokio::task::JoinHandle<()>,
}

impl Capture {
    pub(super) fn begin(id: Option<String>, deadline: Instant) -> Self {
        let report = Arc::new(Mutex::new(Report::new()));
        let stop = CancellationToken::new();
        let output = report.clone();
        let stopping = stop.clone();
        let task = tokio::spawn(async move {
            let observed = observe(id, output.clone(), stopping.clone());
            tokio::select! {
                result = tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), observed) => {
                    match result {
                        Ok(Ok(())) => {},
                        Ok(Err(failure)) => output.lock().unwrap().failed(failure),
                        Err(_) => output.lock().unwrap().stage = "existing_test_deadline",
                    }
                }
                () = stopping.cancelled() => output.lock().unwrap().stage = "observation_stopped",
            }
        });
        Self { report, stop, task }
    }

    pub(super) fn finish(self) {
        self.stop.cancel();
        // Cancel sampling at the assertion point without waiting for another sample. This
        // adds no grace to the existing deadline and emits only the fixed typed fields.
        self.task.abort();
        let mut report = self.report.lock().unwrap();
        report.stage = match report.stage {
            "pending" => "stopped_before_capture",
            "sampling" => "stopped_at_assertion",
            stage => stage,
        };
        eprintln!("OOM_LIVE_EVIDENCE {}", serde_json::to_string(&*report).unwrap());
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.stop.cancel();
        self.task.abort();
    }
}

#[derive(PartialEq, Eq)]
struct Identity {
    id: String,
    start_unix_ns: i128,
    pid: i32,
}

fn identity(inspect: &ContainerInspectResponse, expected_id: &str) -> Result<Identity, Failure> {
    let state = inspect.state.as_ref().ok_or(Failure::invalid("missing_engine_state"))?;
    let id =
        inspect.id.as_ref().filter(|id| id.as_str() == expected_id).ok_or(Failure::invalid("engine_id_changed"))?;
    let raw_start = state.started_at.as_deref().ok_or(Failure::invalid("invalid_engine_start"))?;
    // Bound the owner's text before parsing: RFC3339 permits arbitrary fractional digits.
    const MAX_STARTED_AT_BYTES: usize = 64;
    if raw_start.len() > MAX_STARTED_AT_BYTES {
        return Err(Failure::invalid("engine_start_too_long"));
    }
    let start_unix_ns = time::OffsetDateTime::parse(raw_start, &time::format_description::well_known::Rfc3339)
        .map_err(|_| Failure::invalid("invalid_engine_start"))?
        .unix_timestamp_nanos();
    let pid = state
        .pid
        .and_then(|pid| i32::try_from(pid).ok())
        .filter(|pid| *pid > 1)
        .ok_or(Failure::invalid("engine_pid_unavailable"))?;
    if state.running != Some(true) {
        return Err(Failure::invalid("engine_not_running"));
    }
    Ok(Identity { id: id.clone(), start_unix_ns, pid })
}

async fn capture(id: &str, report: &Arc<Mutex<Report>>) -> Result<Group, Failure> {
    let socket = pryxd::config::DockerConfig::default().socket;
    let socket = socket.to_str().ok_or(Failure::invalid("invalid_socket_path"))?;
    let docker = bollard::Docker::connect_with_unix(socket, 1, bollard::API_DEFAULT_VERSION)
        .map_err(|_| Failure::invalid("engine_client_unavailable"))?
        .negotiate_version()
        .await
        .map_err(|_| Failure::invalid("engine_negotiate_unavailable"))?;
    let first = docker.inspect_container(id, None).await.map_err(|_| Failure::invalid("engine_inspect_unavailable"))?;
    let expected = identity(&first, id)?;
    {
        let mut report = report.lock().unwrap();
        report.id = Some(expected.id.clone());
        report.started_at_unix_ns = Some(expected.start_unix_ns);
        report.host_pid = Some(expected.pid);
        report.configured_memory_bytes = first.host_config.and_then(|host| host.memory);
    }
    let process = Process::capture(expected.pid)?;
    {
        let mut report = report.lock().unwrap();
        report.start_ticks = Some(process.ticks);
        report.pidfd_opened = true;
    }
    let group = Group::capture(process, id)?;
    let current =
        docker.inspect_container(id, None).await.map_err(|_| Failure::invalid("engine_confirm_unavailable"))?;
    if identity(&current, id)? != expected {
        return Err(Failure::invalid("engine_start_changed"));
    }
    group.confirm()?;
    {
        let mut report = report.lock().unwrap();
        report.owner_confirmed = true;
        report.cgroup = Some(group.report.clone());
    }
    Ok(group)
}

async fn observe(id: Option<String>, report: Arc<Mutex<Report>>, stop: CancellationToken) -> Result<(), Failure> {
    let id = id.filter(|id| container_id(id)).ok_or(Failure::invalid("missing_immutable_id"))?;
    report.lock().unwrap().id = Some(id.clone());
    let group = tokio::time::timeout(Duration::from_secs(1), capture(&id, &report))
        .await
        .map_err(|_| Failure::invalid("owner_capture_deadline"))??;
    loop {
        let sample = group.sample()?;
        let alive = sample.pidfd_alive;
        {
            let mut report = report.lock().unwrap();
            if report.first_observed.is_none() {
                report.first_observed = Some(sample.clone());
            }
            report.last_observed = Some(sample);
            report.samples += 1;
            report.stage = if alive { "sampling" } else { "process_exited_counter_readable" };
        }
        if !alive {
            return Ok(());
        }
        tokio::select! {
            () = stop.cancelled() => return Ok(()),
            () = tokio::time::sleep(Duration::from_millis(10)) => {},
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bollard::models::ContainerState;

    #[test]
    fn engine_binding_rejects_another_id_pid_or_start() {
        let id = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let first = ContainerInspectResponse {
            id: Some(id.into()),
            state: Some(ContainerState {
                running: Some(true),
                pid: Some(42),
                started_at: Some("2026-11-17T04:59:41.247679723Z".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let expected = identity(&first, id).unwrap_or_else(|failure| panic!("{}", failure.stage));
        let mut other = first.clone();
        other.id = Some("different".into());
        assert!(identity(&other, id).is_err());
        other = first.clone();
        other.state.as_mut().unwrap().pid = Some(43);
        assert!(identity(&other, id).is_ok_and(|actual| actual != expected));
        other = first.clone();
        other.state.as_mut().unwrap().started_at = Some("2026-11-17T04:59:42.247679723Z".into());
        assert!(identity(&other, id).is_ok_and(|actual| actual != expected));
        other.state.as_mut().unwrap().running = Some(false);
        assert!(identity(&other, id).is_err());
    }

    #[test]
    fn engine_start_is_bounded_before_parse_and_stored_numerically() {
        let id = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let mut inspect = ContainerInspectResponse {
            id: Some(id.into()),
            state: Some(ContainerState {
                running: Some(true),
                pid: Some(42),
                started_at: Some("2026-11-17T04:59:41.247679723Z".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let canonical = identity(&inspect, id).unwrap_or_else(|failure| panic!("{}", failure.stage));
        inspect.state.as_mut().unwrap().started_at = Some("2026-11-17T04:59:41.247679723+00:00".into());
        assert!(identity(&inspect, id).is_ok_and(|actual| actual == canonical));
        let overlong = format!("2026-11-17T04:59:41.{}Z", "0".repeat(256));
        assert!(time::OffsetDateTime::parse(&overlong, &time::format_description::well_known::Rfc3339).is_ok());
        inspect.state.as_mut().unwrap().started_at = Some(overlong);
        assert!(identity(&inspect, id).is_err_and(|failure| failure.stage == "engine_start_too_long"));
    }
}
