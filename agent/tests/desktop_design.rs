// SPDX-License-Identifier: FSL-1.1-ALv2
//! Observe unchanged X11 programs under the complete process policy before selecting desktop
//! transport. Fixture acquisition is explicit; this test announces no desktop capability.
use pryx_agent::process_policy::{POLICY_ENV, Policy};
use serde_json::{Value, json};
use std::{path::Path, process::Output, time::Duration};
use tokio::process::Command;

async fn run(root: &Path, name: &str, arguments: &[String], restricted: bool) -> Output {
    let program = root.join("usr/bin").join(name);
    let mut command = if restricted {
        let policy = Policy {
            data: root.into(),
            memory_bytes: 512 * 1024 * 1024,
            address_space_bytes: 64 * 1024 * 1024 * 1024,
            open_files: 256,
            processes: 4096,
            cpu_seconds: 60,
            nice: 1,
            network_off: false,
            tools: None,
            require_ipc_scope: None,
        };
        let mut command = Command::new(env!("CARGO_BIN_EXE_pryx-agent"));
        command.env_clear().arg("restricted").arg(&program).env(POLICY_ENV, serde_json::to_string(&policy).unwrap());
        command
    } else {
        let mut command = Command::new(program);
        command.env_clear();
        command
    };
    command
        .args(arguments)
        .current_dir(root)
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", root)
        .env("DISPLAY", root.join("private/display"))
        .env("LD_LIBRARY_PATH", format!("{}/usr/lib/x86_64-linux-gnu:{}/libraries", root.display(), root.display()))
        .kill_on_drop(true);
    tokio::time::timeout(Duration::from_secs(5), command.output()).await.expect("X11 argument probe deadline").unwrap()
}

#[tokio::test]
#[ignore = "explicit pinned X11 fixture and hosted process-policy design acceptance"]
async fn unchanged_x11_programs_reject_private_path_displays_under_process_policy() {
    let root = std::path::PathBuf::from(
        std::env::var_os("PRYX_DESKTOP_DESIGN_FIXTURE").expect("CI supplies the reviewed X11 fixture"),
    )
    .canonicalize()
    .unwrap();
    let fixture: Value = serde_json::from_slice(&std::fs::read(root.join("fixture.json")).unwrap()).unwrap();
    for name in ["Xvfb", "xdotool"] {
        let bytes = std::fs::read(root.join("usr/bin").join(name)).unwrap();
        let digest = ring::digest::digest(&ring::digest::SHA256, &bytes);
        let actual = digest.as_ref().iter().map(|byte| format!("{byte:02x}")).collect::<String>();
        assert_eq!(actual, fixture["programs"][name]["sha256"].as_str().unwrap());
    }
    let filesystem = pryx_agent::process_capabilities::probe().filesystem;
    if std::env::var_os("PRYX_DESKTOP_DESIGN_REQUIRE_POLICY").is_some() {
        assert!(filesystem, "Hosted design acceptance requires actual filesystem enforcement");
        assert!(!rustix::process::geteuid().is_root());
        assert!(pryx_agent::process_capabilities::private_ipc_probe().supported());
    }
    let mut records = Vec::new();
    for (name, control, arguments) in [
        ("Xvfb", "-help", vec![format!(":{}", root.join("private/display").display())]),
        ("xdotool", "--version", vec!["getmouselocation".into()]),
    ] {
        let baseline_control = run(&root, name, &[control.into()], false).await;
        assert!(baseline_control.status.success(), "{name}: {baseline_control:?}");
        let baseline = run(&root, name, &arguments, false).await;
        assert!(!baseline.status.success(), "Private-path display unexpectedly worked: {name}");
        assert!(!baseline.stderr.is_empty());
        let confined_control = run(&root, name, &[control.into()], true).await;
        let confined = run(&root, name, &arguments, true).await;
        if filesystem {
            assert!(confined_control.status.success(), "{name}: {confined_control:?}");
            assert_eq!(confined_control.stdout, baseline_control.stdout);
            assert_eq!(confined_control.stderr, baseline_control.stderr);
            assert_eq!(confined.status.code(), baseline.status.code());
            assert_eq!(confined.stdout, baseline.stdout);
            assert_eq!(confined.stderr, baseline.stderr);
        } else {
            for output in [&confined_control, &confined] {
                assert!(!output.status.success());
                assert!(
                    String::from_utf8_lossy(&output.stderr)
                        .contains(pryx_agent::process_capabilities::FILESYSTEM_UNAVAILABLE)
                );
            }
        }
        records.push(json!({"program":name,"binarySha256":fixture["programs"][name]["sha256"],
            "controlWorks":true,"privatePathDisplay":false,"baselineExit":baseline.status.code(),
            "baselineReason":String::from_utf8_lossy(&baseline.stderr).lines().next(),
            "confinedResultMatchesRealProgram":filesystem}));
    }
    let report = json!({"uid":rustix::process::geteuid().as_raw(),"filesystemEnforced":filesystem,
        "programs":records,"packages":fixture["packages"],
        "scope":"Xvfb rejects a pathname display before binding; no live X11 endpoint or headful Chromium acceptance is claimed",
        "desktopEnabled":false});
    if let Some(path) = std::env::var_os("PRYX_DESKTOP_DESIGN_REPORT") {
        std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
    println!("{report}");
}
