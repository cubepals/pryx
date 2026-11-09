// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! The real agent protects bootstrap material and preserves ordinary image namespaces.
//! No environment bytes enter diagnostics even if a regression fails.

use pryx_agent::auth::{Claims, Scope, SigningKey};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

struct AgentProcess(Child);
impl Drop for AgentProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn embedded_printf_does_not_depend_on_external_utilities() {
    let dir = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_pryx-agent"))
        .args(["shell", "printf shell-ready"])
        .env_clear()
        .env("PATH", "/nonexistent")
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(output.status.success(), "Embedded command failed: {}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(output.stdout, b"shell-ready");
}

#[test]
fn host_fuse_approval_keeps_ordinary_images_in_their_user_namespace() {
    let dir = tempfile::tempdir().unwrap();
    let held = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = held.local_addr().unwrap().port();
    drop(held);
    let (_, encoded) = SigningKey::generate().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_pryx-agent"))
        .args([
            "--sandbox",
            "ordinary",
            "--epoch",
            "1",
            "--data",
            dir.path().to_str().unwrap(),
            "--domain",
            "example.test",
            "--port",
            &port.to_string(),
            "--secret-env",
            "TEST_ORDINARY_SECRET",
            "--",
            "readlink",
            "/proc/self/ns/user",
        ])
        .env("TEST_ORDINARY_SECRET", encoded)
        .env(pryx_agent::contract::FUSE_ENV, "1")
        .env_remove(pryx_agent::contract::MOUNTS_ENV)
        .env_remove(pryx_agent::tools::defaults::KIND_ENV)
        .env_remove(pryx_agent::process_policy::POLICY_ENV)
        .output()
        .unwrap();
    assert!(output.status.success(), "Ordinary bootstrap failed: {}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        std::fs::read_link("/proc/self/ns/user").unwrap().to_string_lossy()
    );
}

#[tokio::test]
async fn same_user_processes_cannot_read_the_agent_environment() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let dir = tempfile::tempdir().unwrap();
    let held = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = held.local_addr().unwrap().port();
    drop(held);
    let (key, encoded) = SigningKey::generate().unwrap();
    let now = pryx_agent::telemetry::now();
    let token = key
        .sign(&Claims {
            sandbox: "private".into(),
            epoch: 1,
            issued_at: now,
            expires_at: now + 60,
            scope: Scope::Sandbox,
            max_ports: 0,
            max_exec_seconds: Some(10),
            file: None,
        })
        .unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_pryx-agent"));
    command
        .args([
            "--sandbox",
            "private",
            "--epoch",
            "1",
            "--domain",
            "example.test",
            "--port",
            &port.to_string(),
            "--data",
            dir.path().to_str().unwrap(),
            "--secret-env",
            "PRYX_AGENT_SECRET_privacy",
            "--keep-alive",
        ])
        .env("PRYX_AGENT_SECRET_privacy", encoded)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let process = AgentProcess(command.spawn().unwrap());
    let client = reqwest::Client::new();
    let mut ready = false;
    for _ in 0..200 {
        if client
            .get(format!("http://127.0.0.1:{port}/_pryx/health"))
            .bearer_auth(&token)
            .send()
            .await
            .is_ok_and(|r| r.status().is_success())
        {
            ready = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(ready, "Agent did not become ready");
    let attempted = std::fs::read(format!("/proc/{}/environ", process.0.id()));
    assert!(
        matches!(attempted,Err(ref error) if error.kind()==std::io::ErrorKind::PermissionDenied),
        "Agent environment was accessible to another process"
    );
}
