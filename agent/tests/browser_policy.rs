// SPDX-License-Identifier: FSL-1.1-ALv2
//! Actual executable bootstrap discovery and initial browser policy are checked without a
//! browser fixture. Container rendering and CDP remain real engine/edge acceptance cases.
use pryx_agent::{
    auth::SigningKey,
    bootstrap_capabilities::{BROWSER_ARGUMENT, Capabilities, PROBE},
};
use serde_json::{Value, json};
use std::{
    net::{Ipv4Addr, TcpListener},
    process::Command,
};

#[test]
fn actual_agent_announces_the_optional_bootstrap_parser() {
    let output = Command::new(env!("CARGO_BIN_EXE_pryx-agent")).arg(PROBE).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let value: Capabilities = serde_json::from_slice(&output.stdout).unwrap();
    assert!(value.browser_policy);
}

#[test]
fn default_and_explicit_browser_policy_override_request_environment() {
    for policy in [None, Some(true), Some(false)] {
        let data = tempfile::tempdir().unwrap();
        let forged = tempfile::tempdir().unwrap();
        std::fs::write(forged.path().join("browser-access.json"), r#"{"chromiumSandbox":false}"#).unwrap();
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let bindings = json!({port.to_string():[Ipv4Addr::LOCALHOST]}).to_string();
        let (_, key) = SigningKey::generate().unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_pryx-agent"));
        command.args([
            "--sandbox",
            "browser-policy",
            "--epoch",
            "1",
            "--data",
            data.path().to_str().unwrap(),
            "--domain",
            "pryx.test",
            "--port",
            &port.to_string(),
            "--host-bindings",
            &bindings,
            "--secret-env",
            "TEST_BROWSER_SECRET",
        ]);
        if let Some(policy) = policy {
            command.args([format!("--{BROWSER_ARGUMENT}"), policy.to_string()]);
        }
        command
            .args(["--", "/bin/sh", "-c", "cat \"$PRYX_TOOL_DIRECTORY/browser-access.json\""])
            .env("TEST_BROWSER_SECRET", key)
            .env(pryx_agent::contract::TOOL_KIND_ENV, "browser")
            .env(pryx_agent::tools::defaults::DIRECTORY_ENV, forged.path())
            .env("PRYX_BROWSER_SANDBOX", "0")
            .env_remove(pryx_agent::process_policy::POLICY_ENV);
        drop(listener);
        let output = command.output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["chromiumSandbox"], policy.unwrap_or(true));
        assert!(value.get("token").is_none(), "initial policy must authorize no proxy access");
    }
}
