// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Trusted launch setup must not follow guest-created scratch symlinks, even before a namespace
//! helper starts. Missing helper executables keep these regressions independent of host policy.
use pryx_agent::{
    process_policy, process_tool_policy,
    process_tools::{browser, desktop, ssh},
};
use std::{fs, os::unix::fs::symlink, path::Path};

#[test]
fn native_launch_diagnostics_cannot_truncate_a_foreign_file() {
    let root = tempfile::tempdir().unwrap();
    for name in ["data", "cache", "downloads", "scratch"] {
        fs::create_dir(root.path().join(name)).unwrap();
    }
    let sha256 = "a".repeat(64);
    let bundle = root.path().join("cache").join(&sha256);
    fs::create_dir(&bundle).unwrap();
    let policy = process_policy::Policy {
        data: root.path().join("data"),
        memory_bytes: 512 * 1024 * 1024,
        address_space_bytes: process_policy::DEFAULT_ADDRESS_SPACE_MB * 1024 * 1024,
        open_files: 256,
        processes: 4096,
        cpu_seconds: 60,
        nice: 1,
        network_off: false,
        require_ipc_scope: Some(true),
        tools: Some(process_tool_policy::Policy {
            cache: root.path().join("cache"),
            downloads: root.path().join("downloads"),
            scratch: root.path().join("scratch"),
            artifact: Some(process_tool_policy::Artifact {
                url: "https://example.invalid/tools.tar.gz".into(),
                sha256,
                manifest_sha256: "b".repeat(64),
                bytes: 1,
                unpacked_bytes: 1,
                entries: 1,
            }),
            unavailable: None,
            browser_on: true,
            desktop_on: Some(true),
            desktop_unavailable: None,
        }),
    };
    let outside = root.path().join("foreign");
    fs::write(&outside, b"foreign data must survive trusted setup").unwrap();
    for name in ["browser", "desktop", "server"] {
        let scratch = root.path().join("scratch").join(name);
        fs::create_dir(&scratch).unwrap();
        symlink(&outside, scratch.join(format!("{name}.stderr"))).unwrap();
        let result = match name {
            "browser" => browser::start(
                &policy,
                policy.tools.as_ref().unwrap(),
                browser::Launch { scratch, address: "unused".into(), private_token: "unused".into() },
            ),
            "desktop" => desktop::start_at(
                &policy,
                desktop::Launch {
                    scratch,
                    address: "unused".into(),
                    private_token: "unused".into(),
                    application_url: None,
                },
                &bundle,
                Path::new(env!("CARGO_BIN_EXE_pryx-agent")),
            ),
            _ => ssh::start_at(
                &policy,
                ssh::Launch { directory: scratch, address: "unused".into() },
                &bundle,
                Path::new(env!("CARGO_BIN_EXE_pryx-agent")),
            ),
        };
        assert!(result.is_err());
        assert_eq!(fs::read(&outside).unwrap(), b"foreign data must survive trusted setup");
    }
}
