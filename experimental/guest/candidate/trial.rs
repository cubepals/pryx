// SPDX-License-Identifier: FSL-1.1-ALv2
//! Isolated native guest activation experiment. All timed boots use the same trusted
//! filesystem and the production authenticated framed readiness path. Two separate
//! barrier-equipped correctness boots exercise the original queued raw and H2 peers.

use super::{BootPurpose, Guest, PreparedImage, Value, agent_baseline, boot, fs, input, json};
use pryxd::runtime::vm::filesystems::FilesystemTools;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

#[derive(Clone, Copy)]
pub(super) enum Case {
    Stock,
    Eager,
    Activated,
    FirstRaw,
    FirstFramed,
}

impl Case {
    fn label(self) -> &'static str {
        match self {
            Self::Stock => "stock",
            Self::Eager => "eager",
            Self::Activated => "activated",
            Self::FirstRaw => "first-raw",
            Self::FirstFramed => "first-framed",
        }
    }

    pub(super) fn functional(self) -> bool {
        matches!(self, Self::FirstRaw | Self::FirstFramed)
    }

    pub(super) fn executables(self) -> (&'static str, &'static str) {
        match self {
            Self::Stock => ("/init", "init"),
            Self::Eager => ("/lab-control", "init"),
            _ => ("/lab-init", "lab-service"),
        }
    }

    pub(super) fn inputs(self) -> (PathBuf, PathBuf) {
        let configurator = match self {
            Self::Stock => input("PRYX_VM_CONFIGURATOR"),
            Self::Eager => input("PRYX_LAB_CONFIG_CONTROL"),
            _ => input("PRYX_LAB_CONFIG_ACTIVATION"),
        };
        let image = if self.functional() { "PRYX_LAB_PROOF_BOOTSTRAP" } else { "PRYX_LAB_TRIAL_BOOTSTRAP" };
        (configurator, input(image))
    }

    pub(super) async fn prove_first(self, guest: &Guest, public_key: &str, sandbox: &str, dir: &Path) {
        let mode = match self {
            Self::FirstRaw => "raw",
            Self::FirstFramed => "framed",
            _ => return,
        };
        let message = json!({"socket": guest.channel, "mode": mode, "token": guest.token, "sandbox": sandbox,
                             "epoch": 1, "publicKey": public_key});
        let mut child = Command::new(input("PRYX_LAB_GATE_PROBE"))
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let mut stdin = child.stdin.take().unwrap();
        stdin.write_all(&serde_json::to_vec(&message).unwrap()).await.unwrap();
        drop(stdin);
        let result = child.wait_with_output().await.unwrap();
        assert!(result.status.success(), "The signed first request and simultaneous wrong bearer must both finish");
        let proof: Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(proof.as_object().unwrap().len(), 5);
        assert_eq!(proof["firstStatus"], 200);
        assert_eq!(proof["secondStatus"], 401);
        assert_eq!(proof["identityVerified"], true);
        assert_eq!(proof["mode"], mode);
        assert!(proof["servingPid"].as_u64().is_some_and(|p| p > 1));
        fs::write(dir.join("first.json"), serde_json::to_vec(&proof).unwrap()).unwrap();
    }
}

fn image(dir: &Path) -> Value {
    let description: Value = serde_json::from_slice(&fs::read(dir.join("bootstrap.json")).unwrap()).unwrap();
    let digest = hex::encode(Sha256::digest(fs::read(dir.join("bootstrap.erofs")).unwrap()));
    assert_eq!(description["imageSha256"], digest);
    for key in ["agentSha256", "labControlSha256", "labSupervisorSha256", "labReceiverSha256"] {
        assert_eq!(description[key].as_str().unwrap().len(), 64);
    }
    description
}

async fn sample(case: Case, pair: usize, tools: &FilesystemTools, prepared: &PreparedImage, work: &Path) -> Value {
    let name = format!("{pair:02}-{}", case.label());
    let (dir, writes) = agent_baseline::disk(work, tools, &name).await;
    let acceptance = boot(tools, prepared, &dir, &writes, 0, false, BootPurpose::Candidate(case)).await;
    assert_eq!(acceptance["authenticatedInitShutdown"], true);
    assert_eq!(acceptance["signedCleanShutdown"], true);
    assert_eq!(acceptance["shutdownReplayRefused"], true);
    let path = if case.functional() { "first.json" } else { "baseline-0.json" };
    let observation: Value = serde_json::from_slice(&fs::read(dir.join(path)).unwrap()).unwrap();
    fs::remove_file(writes).unwrap();
    json!({"case":case.label(), "pair":pair, "observation":observation, "acceptance":acceptance})
}

#[tokio::test]
#[ignore = "separate candidate dispatch after the stock baseline; requires an ordinary-user KVM monitor"]
async fn authenticated_candidate() {
    let (tools, work, prepared) = agent_baseline::inputs().await;
    let (_, timing_image) = Case::Stock.inputs();
    let (_, barrier_image) = Case::FirstRaw.inputs();
    let timed = image(&timing_image);
    let barrier = image(&barrier_image);
    for fixed in [
        "agentSha256",
        "labControlSha256",
        "labReceiverSha256",
        "metadata",
        "filesystemManifestSha256",
        "networkHelperSha256",
        "packetForwarderSha256",
        "networkProbe",
    ] {
        assert_eq!(timed[fixed], barrier[fixed]);
    }
    assert_ne!(timed["labSupervisorSha256"], barrier["labSupervisorSha256"]);
    let mut first = Vec::new();
    for case in [Case::FirstRaw, Case::FirstFramed] {
        first.push(sample(case, 0, &tools, &prepared, &work).await);
    }
    let mut samples = Vec::new();
    let mut order = [Case::Stock, Case::Eager, Case::Activated];
    for pair in 1..=15 {
        let direction = if pair % 2 == 1 { "forward" } else { "reverse" };
        for (i, case) in order.iter().enumerate() {
            let mut result = sample(*case, pair, &tools, &prepared, &work).await;
            result["position"] = (i + 1).into();
            result["direction"] = direction.into();
            samples.push(result);
        }
        order.reverse();
        let report = json!({"complete": pair == 15, "ociImage": prepared.digest, "bootstrap": timed,
                            "correctnessBootstrap": barrier, "firstRequests": first,
                            "ordering":{"oddRounds":["stock","eager","activated"],"evenRounds":["activated","eager","stock"],
                                        "forwardRounds":8,"reverseRounds":7,"positionBase":1},
                            "readinessTransport":"stock framed management forwarder", "samples":samples});
        fs::write(work.join("candidate-samples.json"), serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
}
