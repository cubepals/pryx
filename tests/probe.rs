// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Real kernel and service observations through the shipped command. CI additionally requires
//! a successful KVM guest on its hosted runner; device absence is tested on ordinary images.
use pryxd::probe::{Report, State};
use std::process::Command;

#[test]
fn the_probe_reports_every_capability_without_a_configuration_or_secrets() {
    let output = Command::new(env!("CARGO_BIN_EXE_pryxd"))
        .args(["probe", "--json"])
        .env("PRYX_PROBE_SECRET_CANARY", "probe-must-not-leak-this-value")
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(!text.contains("probe-must-not-leak-this-value"));
    let report: Report = serde_json::from_str(&text).unwrap();
    assert_eq!(report.architecture, std::env::consts::ARCH);
    for name in [
        "landlock",
        "user-namespaces",
        "cgroup-controllers",
        "kvm",
        "kvm-device",
        "fuse-device",
        "docker",
        "docker-user",
        "podman",
        "podman-user",
        "host-firewall",
        "systemd",
        "user-session",
        "linger",
    ] {
        let capability = &report.capabilities[name];
        assert!(!capability.evidence.is_empty(), "{name}");
        assert!(!capability.action.is_empty(), "{name}");
    }
    let namespace = Command::new(env!("CARGO_BIN_EXE_pryxd")).arg("probe-user-namespace").output().unwrap();
    assert_eq!(report.on("user-namespaces"), namespace.status.success());
    for name in ["kvm", "fuse"] {
        let exists = std::path::Path::new(&format!("/dev/{name}")).try_exists().unwrap();
        if !exists {
            assert_eq!(report.capabilities[&format!("{name}-device")].state, State::Off);
        }
    }
    if std::env::var("PRYX_PROBE_KVM").as_deref() == Ok("on") {
        assert!(report.on("kvm"), "{:?}", report.capabilities["kvm"]);
        assert!(report.capabilities["kvm"].evidence.contains("HLT exits"));
    }
}
