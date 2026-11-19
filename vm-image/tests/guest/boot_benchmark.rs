// SPDX-License-Identifier: FSL-1.1-ALv2
//! Paired ordinary-user boot timings and the pinned monitor's balloon reporting counters.

use super::{FilesystemTools, Guest, PreparedImage, boot, request};
use axum::http::StatusCode;
use serde_json::{Value, json};
use std::{
    fs,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::time::Instant;

const PAIRS: usize = 8;
const REPORT_WINDOW: Duration = Duration::from_secs(12);
const FLUSH_INTERVAL: Duration = Duration::from_millis(500);
const FLUSH_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Copy)]
pub(super) enum Profile {
    Baseline,
    Quiet,
    Reporting,
}

impl Profile {
    pub(super) fn configuration(self, bytes: Vec<u8>, directory: &Path, round: usize) -> (Vec<u8>, Option<PathBuf>) {
        let mut config: Value = serde_json::from_slice(&bytes).unwrap();
        let metrics = match self {
            Self::Baseline => return (bytes, None),
            Self::Quiet => {
                let arguments = config["boot-source"]["boot_args"].as_str().unwrap();
                config["boot-source"]["boot_args"] = quiet(arguments).into();
                None
            }
            Self::Reporting => {
                let path = directory.join(format!("metrics{round}.jsonl"));
                fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path).unwrap();
                // Firecracker 1.18's BalloonDeviceConfig accepts free_page_reporting.
                // The generated SDK model describes only amount, deflate and polling.
                config["balloon"] = json!({"amount_mib": 0, "deflate_on_oom": false, "free_page_reporting": true});
                config["metrics"] = json!({"metrics_path": path});
                Some(path)
            }
        };
        (serde_json::to_vec(&config).unwrap(), metrics)
    }
}

fn quiet(arguments: &str) -> String {
    let (kernel, init) = arguments.split_once(" -- ").expect("Guest init arguments follow the kernel separator");
    format!("{kernel} quiet -- {init}")
}

#[derive(Default)]
struct Counters {
    reports: u64,
    bytes: u64,
    failures: u64,
}

impl Counters {
    fn from_log(log: &str) -> Self {
        let mut total = Self::default();
        // A flush reports counter deltas. Ignore a trailing row still being written.
        let complete = log.rsplit_once('\n').map_or("", |(complete, _)| complete);
        for line in complete.lines() {
            let metric: Value = serde_json::from_str(line).expect("Expected one Firecracker metrics object per line");
            let balloon = &metric["balloon"];
            total.reports += balloon["free_page_report_count"].as_u64().unwrap();
            total.bytes += balloon["free_page_report_freed"].as_u64().unwrap();
            total.failures += balloon["free_page_report_fails"].as_u64().unwrap();
        }
        total
    }

    fn observed(&self) -> bool {
        self.reports > 0 && self.bytes > 0
    }
}

pub(super) async fn observe_reports(guest: &Guest, api: &Path, metrics: &Path) -> Value {
    // Touch ordinary guest tmpfs memory and release it for the kernel reporter.
    guest
        .command(vec![
            "/bin/sh".into(),
            "-c".into(),
            "dd if=/dev/zero of=/tmp/pryx-report-pages bs=1048576 count=24 && rm /tmp/pryx-report-pages".into(),
        ])
        .await;
    let started = Instant::now();
    let deadline = started + REPORT_WINDOW;
    let counters = loop {
        let (status, _) = request(
            &guest.client,
            json!({"socket": api, "monitor": {"action": "flushMetrics", "timeoutMs": FLUSH_TIMEOUT.as_millis()}}),
        )
        .await
        .expect("Flush Firecracker's real balloon counters");
        assert_eq!(status, StatusCode::NO_CONTENT);
        let counters = Counters::from_log(&fs::read_to_string(metrics).unwrap());
        if counters.observed() || Instant::now() >= deadline {
            break counters;
        }
        tokio::time::sleep_until((Instant::now() + FLUSH_INTERVAL).min(deadline)).await;
    };
    let state = if counters.observed() { "verified" } else { "unconfirmed" };
    json!({"state": state, "reportDescriptors": counters.reports, "discardRangeBytes": counters.bytes,
        "reportFailures": counters.failures, "observationWindowMs": started.elapsed().as_millis()})
}

fn percentile(values: &[f64], fraction: f64) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    sorted[(sorted.len() as f64 * fraction).ceil() as usize - 1]
}

fn summary(values: &[f64]) -> Value {
    json!({"samplesMs": values, "p50Ms": percentile(values, 0.5), "p95Ms": percentile(values, 0.95)})
}

pub(super) async fn measure(
    tools: &FilesystemTools,
    image: &PreparedImage,
    directory: &Path,
    writes: &Path,
    work: &Path,
    pins: Value,
) {
    assert!(!rustix::process::geteuid().is_root(), "Measure the ordinary-user launch");
    let mut baseline = Vec::with_capacity(PAIRS);
    let mut quiet = Vec::with_capacity(PAIRS);
    let mut savings = Vec::with_capacity(PAIRS);
    let warmup = boot(tools, image, directory, writes, 0, true, Some(Profile::Baseline)).await;
    let kernel = warmup["kernel"].as_str().unwrap();
    for pair in 0..PAIRS {
        let profiles =
            if pair % 2 == 0 { [Profile::Baseline, Profile::Quiet] } else { [Profile::Quiet, Profile::Baseline] };
        for (index, profile) in profiles.into_iter().enumerate() {
            let run = boot(tools, image, directory, writes, 2 * pair + index + 1, true, Some(profile)).await;
            let elapsed = run["readyMs"].as_f64().unwrap();
            assert!(elapsed.is_finite() && elapsed > 0.0);
            let running_kernel = run["kernel"].as_str().unwrap();
            assert_eq!(kernel, running_kernel);
            match profile {
                Profile::Baseline => baseline.push(elapsed),
                Profile::Quiet => quiet.push(elapsed),
                Profile::Reporting => unreachable!(),
            }
        }
        savings.push(baseline[pair] - quiet[pair]);
    }
    let page_reporting = if pins["pageReportingBuiltIn"] != true {
        json!({"state": "unavailable", "reason": "The pinned guest kernel does not set CONFIG_PAGE_REPORTING=y"})
    } else if pins["balloonBuiltIn"] != true {
        json!({"state": "unavailable", "reason": "The pinned guest kernel lacks a built-in virtio balloon driver"})
    } else {
        let result = boot(tools, image, directory, writes, 2 * PAIRS + 1, true, Some(Profile::Reporting)).await;
        assert_eq!(result["kernel"].as_str(), Some(kernel));
        result["pageReports"].clone()
    };
    let report = json!({
        "pins": pins, "kernel": kernel, "imageManifestDigest": image.digest,
        "profile": "ordinary user, prepared OCI image, vsock network", "warmupBoots": 1,
        "interval": "monitor launch to authenticated guest agent readiness",
        "baseline": summary(&baseline), "quiet": summary(&quiet), "pairedSavings": summary(&savings),
        "pageReporting": page_reporting,
    });
    fs::write(work.join("vm-boot-benchmark.json"), serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    let mut markdown = format!(
        "## Ordinary-user VM boot benchmark\n\nSource `{}`. Kernel `{kernel}`. Timed from monitor launch to authenticated guest agent readiness.\n\n| Metric | Pairs | p50 ms | p95 ms |\n| --- | ---: | ---: | ---: |\n",
        report["pins"]["source"].as_str().unwrap(),
    );
    for (label, values) in [("Baseline boot", &baseline), ("Quiet boot", &quiet), ("Paired savings", &savings)] {
        markdown.push_str(&format!(
            "| {label} | {PAIRS} | {:.2} | {:.2} |\n",
            percentile(values, 0.5),
            percentile(values, 0.95)
        ));
    }
    markdown.push_str("\n### Free-page reporting\n\n");
    if let Some(reason) = page_reporting["reason"].as_str() {
        markdown.push_str(reason);
        markdown.push_str(".\n");
    } else {
        markdown.push_str(&format!(
            "{} with {} report descriptors, {} bytes in successful discarded ranges and {} failures.\n",
            page_reporting["state"].as_str().unwrap(),
            page_reporting["reportDescriptors"],
            page_reporting["discardRangeBytes"],
            page_reporting["reportFailures"],
        ));
    }
    fs::write(work.join("vm-boot-benchmark.md"), markdown).unwrap();
}

#[test]
fn quiet_stays_in_the_kernel_arguments() {
    assert_eq!(quiet("console=ttyS0 ro -- trusted-init"), "console=ttyS0 ro quiet -- trusted-init");
}

#[test]
fn reporting_requires_positive_count_and_discarded_bytes() {
    let log = concat!(
        "{\"balloon\":{\"free_page_report_count\":2,\"free_page_report_freed\":0,\"free_page_report_fails\":2}}\n",
        "{\"balloon\":{\"free_page_report_count\":1,\"free_page_report_freed\":4096,\"free_page_report_fails\":0}}\n",
        "{\"balloon\":",
    );
    assert!(!Counters { reports: 1, ..Counters::default() }.observed());
    assert!(!Counters { bytes: 4096, ..Counters::default() }.observed());
    let counters = Counters::from_log(log);
    assert_eq!((counters.reports, counters.bytes, counters.failures), (3, 4096, 2));
    assert!(counters.observed());
}
