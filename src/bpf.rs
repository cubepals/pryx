// SPDX-License-Identifier: FSL-1.1-ALv2
//! Kernel network policy for process sandboxes: small cgroup programs (`bpf/`) that allow,
//! refuse or redirect each destination by the rules pryxd writes, refuse the socket kinds they
//! can't police, and count traffic. Where this daemon may not attach programs to a cgroup of its
//! own, the guarantee is off with the reason and nothing is approximated. The process runtime
//! decides when a sandbox gets them (`runtime/process/network.rs`); this module is the mechanism.
//!
//! Parts (`bpf/`):
//! - `object.rs`: the embedded object, every program and map by name, and the bytes the maps hold.
//! - `rules.rs`: a sandbox's policy as the prefix rules the programs read.
//! - `host.rs`: this daemon's cgroup v2 directory, its BPF filesystem, the host's addresses and
//!   the kernel's word when they change.
//! - `attach.rs`: loading, attaching, pinning, adopting and detaching one sandbox's programs.
pub(crate) mod attach;
pub(crate) mod host;
pub(crate) mod object;
pub(crate) mod rules;

use crate::capability::{Capability, State};

/// The host capability's name in the probe document and in doctor.
pub(crate) const CAPABILITY: &str = "kernel-network-policy";

/// What a proof of the capability found: where the programs go, or why they can't.
pub(crate) fn probe() -> Result<host::Host, host::Unavailable> {
    probe_object(object::OBJECT)
}

/// The proof with `programs` in place of the embedded object: only tests hand it another.
pub(crate) fn probe_object(programs: &[u8]) -> Result<host::Host, host::Unavailable> {
    let host = host::Host::find()?;
    // A sandbox's rules name this host's addresses, so they are kept as the host changes.
    drop(host::subscribe().map_err(|problem| host::Unavailable {
        evidence: format!("this daemon can't hear of changes to the host's addresses (rtnetlink): {problem}"),
        action: host::WATCH_ACTION,
    })?);
    let key = format!("probe-{}", std::process::id());
    let cgroup = host.cgroup(&key);
    let pins = host.pin_directory(&key);
    let fail = |evidence: String, action| host::Unavailable { evidence, action };
    host::create_cgroup(&cgroup).map_err(|problem| {
        fail(
            format!("making a cgroup in {}: {problem}", host.cgroups.display()),
            "Delegate this daemon's cgroup to its user",
        )
    })?;
    let rules = rules::compile(&rules::Inputs {
        reach: rules::Reach::None,
        allow: &[],
        deny: &[],
        host: &[],
        nameservers: &[],
        served: 0..=0,
        api_port: 0,
        private: &[],
        link_local: &[],
        metadata: &[],
        dns: 0,
    })
    .map_err(|problem| fail(problem, "Report this as a bug"));
    let config = object::config_bytes(std::net::Ipv4Addr::LOCALHOST, 0, 0, 0, &[])
        .ok_or_else(|| fail("the probe's configuration does not fit".into(), "Report this as a bug"));
    let attached = rules.and_then(|rules| {
        attach::Programs::attach(programs, &pins, &cgroup, &rules, config?).map_err(|problem| {
            let action = if problem.starts_with(attach::REJECTED) {
                "Report the verifier's words above and this kernel's version as a pryx bug"
            } else {
                "Run as root, or grant this daemon CAP_BPF and CAP_NET_ADMIN on a kernel with cgroup BPF links (5.7 or later)"
            };
            fail(problem, action)
        })
    });
    let joined = attached.and_then(|programs| {
        let joined = join_probe(&cgroup).map_err(|problem| {
            fail(
                format!("moving a process into {}: {problem}", cgroup.display()),
                "Delegate this daemon's cgroup to its user",
            )
        });
        programs.detach().map_err(|problem| fail(problem, "Check the BPF filesystem's permissions"))?;
        joined
    });
    let removed = host::remove_cgroup(&cgroup);
    joined?;
    removed.map_err(|problem| fail(format!("removing {}: {problem}", cgroup.display()), "Check cgroup permissions"))?;
    Ok(host)
}

/// A short process started into the probe's cgroup the way a sandbox's supervisor is.
fn join_probe(cgroup: &std::path::Path) -> std::io::Result<()> {
    let program = ["/usr/bin/true", "/bin/true"]
        .into_iter()
        .find(|path| std::path::Path::new(path).exists())
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no true(1) to start"))?;
    let mut command = std::process::Command::new(program);
    host::join_on_exec(&mut command, cgroup)?;
    let status = command.status()?;
    if status.success() { Ok(()) } else { Err(std::io::Error::other(format!("true(1) exited {status}"))) }
}

/// The probe as the host document reports it.
pub(crate) fn capability(found: &Result<host::Host, host::Unavailable>) -> Capability {
    match found {
        Ok(host) => Capability::on(format!(
            "programs load, attach and detach on kernel {}; {}",
            rustix::system::uname().release().to_string_lossy(),
            host.describe()
        )),
        Err(problem) => Capability::new(State::Off, problem.evidence.clone(), problem.action),
    }
}

#[cfg(test)]
mod tests {
    /// Needs a host that grants the capability: CI runs it as root and as a delegated user.
    #[test]
    #[ignore = "loads programs into the kernel"]
    fn the_probe_loads_attaches_and_removes_every_program() {
        let host = super::probe().unwrap_or_else(|problem| panic!("{}: {}", problem.evidence, problem.action));
        let leftover = std::fs::read_dir(&host.cgroups)
            .unwrap()
            .filter_map(Result::ok)
            .any(|entry| entry.file_name().to_string_lossy() == format!("pryx-probe-{}", std::process::id()));
        assert!(!leftover, "the probe's cgroup was left behind");
    }
}
