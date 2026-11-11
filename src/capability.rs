// SPDX-License-Identifier: FSL-1.1-ALv2
//! Read-only host evidence from bounded child probes and the host C library. Capability policy,
//! configuration and runtime selection belong to callers; this module starts no workload.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    On,
    Off,
    Unknown,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Capability {
    pub state: State,
    pub evidence: String,
    pub action: String,
}
impl Capability {
    pub(crate) fn new(state: State, evidence: impl Into<String>, action: impl Into<String>) -> Self {
        Self { state, evidence: evidence.into(), action: action.into() }
    }
    pub(crate) fn on(evidence: impl Into<String>) -> Self {
        Self::new(State::On, evidence, "No action needed")
    }
    pub(crate) fn error(error: std::io::Error, action: &str) -> Self {
        let state = if error.kind() == std::io::ErrorKind::NotFound { State::Off } else { State::Unknown };
        Self::new(state, error.to_string(), action)
    }
}

pub(crate) fn child(argument: &str) -> Capability {
    use std::{
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    let result = (|| -> std::io::Result<Capability> {
        let mut child = Command::new(std::env::current_exe()?)
            .arg(argument)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if child.try_wait()?.is_some() {
                let output = child.wait_with_output()?;
                if output.status.success() {
                    return Ok(Capability::on(String::from_utf8_lossy(&output.stdout).trim()));
                }
                return Ok(Capability::new(
                    State::Unknown,
                    String::from_utf8_lossy(&output.stderr).trim(),
                    if argument == "probe-kvm" {
                        "Grant this user access to /dev/kvm on a KVM host"
                    } else if argument == "probe-separate-users" {
                        "Grant the server's process runtime SETUID and SETGID for its uid pool"
                    } else {
                        "Allow unprivileged user namespaces and the binary's AppArmor namespace policy"
                    },
                ));
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Ok(Capability::new(
                    State::Unknown,
                    "Probe exceeded two seconds",
                    "Check host virtualization or namespace policy",
                ));
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    })();
    result.unwrap_or_else(|error| Capability::error(error, "Check executable and host permissions"))
}

pub(crate) fn c_library() -> String {
    for (command, args) in [("getconf", vec!["GNU_LIBC_VERSION"]), ("ldd", vec!["--version"])] {
        probe_phase(command);
        if let Ok(output) = std::process::Command::new(command).args(args).output() {
            let bytes = if output.stdout.is_empty() { output.stderr } else { output.stdout };
            let text = String::from_utf8_lossy(&bytes);
            if !text.trim().is_empty() && (output.status.success() || command == "ldd") {
                return text.lines().take(2).collect::<Vec<_>>().join("; ");
            }
        }
    }
    "unknown: neither getconf nor ldd could identify the host C library".into()
}

/// Opt-in fixed phase metadata for a bounded probe's caller; never command arguments or output.
pub(crate) fn probe_phase(phase: &str) {
    if std::env::var("PRYX_PROBE_DIAGNOSTICS").as_deref() == Ok("1") {
        let now = rustix::time::clock_gettime(rustix::time::ClockId::Monotonic);
        eprintln!("PRYX_PROBE phase={phase} monotonic_seconds={} nanos={}", now.tv_sec, now.tv_nsec);
    }
}
