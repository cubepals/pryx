// SPDX-License-Identifier: FSL-1.1-ALv2
//! The native bundle uses host glibc; this prerequisite is independent of tool readiness.
use super::{Capability, State};

pub(crate) fn prerequisite(architecture: &str, host: &str) -> Capability {
    let policy: serde_json::Value =
        serde_json::from_str(include_str!("../../templates/process-tools/compatibility.json"))
            .expect("committed bundle compatibility");
    let target = policy["architecture"].as_str().expect("bundle architecture");
    let minimum = policy["hostGlibcMinimum"].as_str().expect("bundle glibc minimum");
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
        Capability::new(
            State::Off,
            format!("Native process tools require host glibc >= {minimum}; observed {host}"),
            action,
        )
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
}
