// SPDX-License-Identifier: FSL-1.1-ALv2
//! The native bundle uses host glibc; this prerequisite is independent of tool readiness.
use super::{Capability, State};

pub(crate) fn prerequisite(architecture: &str, host: &str) -> Capability {
    let policy: serde_json::Value =
        serde_json::from_str(include_str!("../../templates/process-tools/compatibility.json"))
            .expect("committed bundle compatibility");
    let target = policy["architecture"].as_str().expect("bundle architecture");
    let minimum = policy["hostGlibcMinimum"].as_str().expect("bundle glibc minimum");
    for_minimum(architecture, host, target, minimum, "Native process tools")
}

pub(crate) fn desktop_prerequisite(architecture: &str, host: &str) -> Capability {
    let policy: serde_json::Value =
        serde_json::from_str(include_str!("../../templates/process-tools/compatibility.json"))
            .expect("committed bundle compatibility");
    let desktop: serde_json::Value =
        serde_json::from_str(include_str!("../../templates/process-tools/desktop-full.json"))
            .expect("committed desktop compatibility");
    for_minimum(
        architecture,
        host,
        policy["architecture"].as_str().expect("bundle architecture"),
        desktop["hostGlibcMinimum"].as_str().expect("desktop host minimum"),
        "Native process desktop tools",
    )
}

pub(crate) fn desktop_capability(ready: &Result<(), String>) -> Capability {
    match ready {
        Ok(()) => Capability::on("Native process desktop bundle, host glibc and private namespaces are available"),
        Err(reason) => Capability::new(
            State::Off,
            reason.clone(),
            "Inspect native desktop bundle, glibc, private IPC and user-namespace policy",
        ),
    }
}

pub(crate) fn ssh_capability(ready: &Result<(), String>, supported: bool) -> Capability {
    match ready {
        Err(reason) => Capability::new(
            State::Off,
            format!("Native SSH prerequisite: {reason}"),
            "Inspect the native OpenSSH component, glibc, private IPC and user namespaces",
        ),
        Ok(()) if supported => {
            Capability::on("Native OpenSSH bundle, compatible agent and private process transport are available")
        }
        Ok(()) => Capability::new(
            State::Off,
            "The installed process agent does not announce native SSH support",
            "Upgrade the installed agent to a release providing native SSH",
        ),
    }
}

fn for_minimum(architecture: &str, host: &str, target: &str, minimum: &str, tool: &str) -> Capability {
    let action = format!("Use a {target} host with glibc {minimum} or newer for native process tools");
    if architecture != target {
        return Capability::new(
            State::Off,
            format!("Process tool bundle supports {target}; host is {architecture}"),
            action,
        );
    }
    let observed = (host.contains("glibc") || host.contains("GLIBC") || host.contains("GNU libc"))
        .then(|| version(host))
        .flatten();
    if observed.is_some_and(|observed| observed >= version(minimum).expect("bundle minimum version")) {
        Capability::on(format!(
            "Host {host} meets process-tool glibc {minimum}; tools still require their own readiness"
        ))
    } else {
        Capability::new(State::Off, format!("{tool} require host glibc >= {minimum}; observed {host}"), action)
    }
}

fn version(text: &str) -> Option<(u32, u32)> {
    // A numeric token in musl's banner must never be treated as a GNU libc observation.
    if text.contains("musl") {
        return None;
    }
    text.split_whitespace().find_map(|token| {
        let mut parts = token.split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next()?.parse().ok()?;
        Some((major, minor))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_and_non_glibc_hosts_keep_the_tools_off_with_the_observed_reason() {
        for host in ["glibc 2.9", "glibc 2.27", "glibc 2.33", "musl libc; Version 2.40", "unknown"] {
            let answer = prerequisite("x86_64", host);
            assert_eq!(answer.state, State::Off);
            assert!(answer.evidence.contains(host));
            assert!(answer.evidence.contains("2.34"));
        }
        for host in ["glibc 2.34", "glibc 2.41", "ldd (Debian GLIBC 2.36-9) 2.36"] {
            assert_eq!(prerequisite("x86_64", host).state, State::On);
        }
        assert_eq!(prerequisite("aarch64", "glibc 2.41").state, State::Off);
    }

    #[test]
    fn desktop_floor_does_not_withdraw_usable_core_tools() {
        for host in ["glibc 2.34", "glibc 2.35"] {
            assert_eq!(prerequisite("x86_64", host).state, State::On);
            let desktop = desktop_prerequisite("x86_64", host);
            assert_eq!(desktop.state, State::Off);
            assert!(desktop.evidence.contains(host));
            assert!(desktop.evidence.contains("2.36"));
        }
        assert_eq!(desktop_prerequisite("x86_64", "glibc 2.36").state, State::On);
        assert_eq!(desktop_prerequisite("x86_64", "musl libc; Version 2.40").state, State::Off);
    }
}
