// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2
//! Record guest PID 1 and the serving agent's own RSS immediately after
//! authenticated health succeeds. Execute the stock guest acceptance with a
//! fresh disk each time and retain its signed shutdown checks.

use super::{FilesystemTools, Guest, Images, Method, Run, StatusCode, Value, boot, fs, input, json};
use std::os::unix::fs::DirBuilderExt;
use std::path::Path;
use std::time::{Duration, Instant};

pub(super) async fn observe(guest: &Guest, launched: Instant, phases: [Duration; 4], directory: &Path, round: usize) {
    let [launch_return, forwarded, delivery, ready] = phases;
    // The authenticated exec shell is a direct child of the serving agent.
    // Check its executable and parent before attributing the parent's resident pages.
    let executable = format!("{}/init", pryx_agent::vm_boot::BOOTSTRAP_ROOT);
    let sample = r#"set -eu
test "$PPID" -gt 1
awk '/^VmRSS:/ && $3 == "kB" {print $2}' /proc/1/status /proc/$PPID/status > /data/lab-rss
test "$(tr '\000' '\n' < /proc/$PPID/cmdline | sed -n 1p)" = "$1"
test "$(awk '/^PPid:/ {print $2}' /proc/$PPID/status)" = 1
"#;
    guest.command(vec!["/bin/sh".into(), "-c".into(), sample.into(), "rss".into(), executable]).await;
    let rss_exec_completed = launched.elapsed();
    let (status, data) = guest.call(Method::GET, "/_pryx/files/content?path=lab-rss", Value::Null, None).await;
    assert_eq!(status, StatusCode::OK);
    let rss = String::from_utf8(data).unwrap();
    let values: Vec<u64> = rss.lines().map(|line| line.parse().unwrap()).collect();
    assert_eq!(values.len(), 2, "Expected two VmRSS samples reported in KiB");
    assert!(values.iter().all(|rss| *rss > 0));
    let unauthorized = Guest {
        channel: guest.channel.clone(),
        client: guest.client.clone(),
        token: "invalid-lab-health-proof".into(),
        framed: guest.framed.clone(),
    };
    let (status, _) = unauthorized.call(Method::GET, "/_pryx/health", Value::Null, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let row = json!({"launchToMonitorPlanSentNs": launch_return.as_nanos() as u64,
                     "launchToFramedForwarderNs": forwarded.as_nanos() as u64,
                     "framedForwarderToBootstrapDeliveredNs": (delivery - forwarded).as_nanos() as u64,
                     "launchToBootstrapDeliveredNs": delivery.as_nanos() as u64,
                     "launchToAuthenticatedReadyNs": ready.as_nanos() as u64,
                     "bootstrapDeliveredToAuthenticatedReadyNs": (ready - delivery).as_nanos() as u64,
                     "readyToRssExecCompletedNs": (rss_exec_completed - ready).as_nanos() as u64,
                     "guestInitRssKiB": values[0], "guestServingAgentRssKiB": values[1],
                     "invalidBearerHealthRefused": true});
    fs::write(directory.join(format!("baseline-{round}.json")), serde_json::to_vec_pretty(&row).unwrap()).unwrap();
}

#[tokio::test]
#[ignore = "requires the lab baseline dispatch, a staged production guest and real KVM"]
async fn measure_stock_agent() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let component = input("PRYX_VM_FILESYSTEM_COMPONENT");
    let manifest = std::env::var("PRYX_VM_FILESYSTEM_MANIFEST").unwrap();
    let tools = FilesystemTools::verified(&component, &manifest, &input("PRYX_VM_HOST_LAUNCHER")).unwrap();
    let work = input("PRYX_VM_GUEST_WORK");
    fs::DirBuilder::new().mode(0o700).create(&work).unwrap();
    let images = Images::new(work.join("images"), tools.clone()).unwrap();
    let inputs: Vec<Value> = serde_json::from_str(include_str!("../../templates/vm-tools/image-inputs.json")).unwrap();
    let image = images.prepare(inputs[0]["reference"].as_str().unwrap(), Duration::from_secs(120)).await.unwrap();
    let mut samples = Vec::new();
    for i in 0..15 {
        let dir = work.join(format!("sample-{i:02}"));
        fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
        let writes = dir.join("writes.ext4");
        let disk_bytes = 512 * 1024 * 1024;
        fs::File::create_new(&writes).unwrap().set_len(disk_bytes).unwrap();
        let make = Run {
            program: "mke2fs",
            arguments: ["-q", "-F", "-t", "ext4", writes.to_str().unwrap()].map(String::from).to_vec(),
            read: Vec::new(),
            write: vec![writes.clone()],
            file_bytes: disk_bytes,
        };
        assert!(tools.run(make, &work, None, 64 * 1024).await.unwrap().status.success());
        let acceptance = boot(&tools, &image, &dir, &writes, 0, false, super::BootPurpose::AgentBaseline).await;
        assert_eq!(acceptance["signedCleanShutdown"], true);
        let observation: Value = serde_json::from_slice(&fs::read(dir.join("baseline-0.json")).unwrap()).unwrap();
        samples.push(json!({"sample": i + 1, "observation": observation, "acceptance": acceptance}));
        fs::remove_file(writes).unwrap();
        let report = json!({"image": image.digest, "healthRetryIntervalMs": pryxd::runtime::vm::instance::lab_framed_health::RETRY.as_millis() as u64,
                            "readinessTransport": "stock framed management forwarder",
                            "rssObservationTransport": "stock framed management forwarder",
                            "samples": samples, "complete": samples.len() == 15});
        fs::write(work.join("samples.json"), serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
}
