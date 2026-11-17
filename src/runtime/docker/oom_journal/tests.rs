// SPDX-License-Identifier: FSL-1.1-ALv2
//! Causal journal controls use the production start and inspect paths against an authenticated
//! local engine transport. No control claims that the local kernel enforces a memory limit.
use super::*;
use crate::runtime::ContainerRuntime;
use async_trait::async_trait;
use axum::{Router, extract::Request, routing::any};
use serde_json::{Value, json};
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

const ID: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const OLD: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const NEW: &str = "cccccccccccccccccccccccccccccccc";

#[derive(Default)]
struct Journal {
    visible: Mutex<Vec<Value>>,
    queued: Mutex<Vec<Value>>,
    syncs: AtomicUsize,
    malformed: std::sync::atomic::AtomicBool,
    unavailable: std::sync::atomic::AtomicBool,
    reuse_started: std::sync::atomic::AtomicBool,
    runc: std::sync::atomic::AtomicBool,
    server_failure: std::sync::atomic::AtomicBool,
}

fn record(message: &str, invocation: &str, mono: u64) -> Value {
    json!({"MESSAGE_ID": message, "UNIT": format!("docker-{ID}.scope"), "INVOCATION_ID": invocation,
        "_BOOT_ID": boot().unwrap(), "_PID": "1", "_UID": "0", "_TRANSPORT": "journal",
        "__MONOTONIC_TIMESTAMP": mono.to_string()})
}

#[async_trait]
impl reader::Source for Journal {
    async fn available(&self) -> Result<(), RuntimeError> {
        if self.unavailable.load(Ordering::SeqCst) { Err(failure("fixture journal unavailable")) } else { Ok(()) }
    }

    async fn run(&self, args: &[String]) -> Result<Vec<u8>, RuntimeError> {
        if args.iter().any(|arg| arg == "--sync") {
            self.syncs.fetch_add(1, Ordering::SeqCst);
            let mut queued = self.queued.lock().unwrap();
            for mut record in queued.drain(..) {
                record["__MONOTONIC_TIMESTAMP"] = monotonic().unwrap().to_string().into();
                self.visible.lock().unwrap().push(record);
            }
            return Ok(Vec::new());
        }
        assert!(args.iter().any(|arg| arg == "--lines=2"));
        assert!(args.iter().any(|arg| arg == "_PID=1"));
        assert!(!args.iter().any(|arg| arg.contains("--since") || arg.contains("MESSAGE=")));
        if self.malformed.load(Ordering::SeqCst) {
            return Ok(b"{truncated".to_vec());
        }
        let records = self.visible.lock().unwrap();
        let selected = records
            .iter()
            .filter(|record| {
                args.iter()
                    .filter_map(|arg| arg.split_once('='))
                    .filter(|(name, _)| !name.starts_with('-'))
                    .all(|(name, value)| record.get(name).and_then(Value::as_str) == Some(value))
            })
            .rev()
            .take(2)
            .collect::<Vec<_>>();
        Ok(selected
            .into_iter()
            .rev()
            .map(|record| serde_json::to_string(record).unwrap() + "\n")
            .collect::<String>()
            .into_bytes())
    }
}

struct Fixture {
    directory: tempfile::TempDir,
    socket: PathBuf,
    journal: Arc<Journal>,
    state: Arc<Mutex<(String, bool)>>,
    calls: Arc<AtomicUsize>,
    server: tokio::task::JoinHandle<()>,
}

impl Fixture {
    fn new(suppressed: bool, failed: bool, multiple: bool) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let socket = directory.path().join("engine.sock");
        let listener = tokio::net::UnixListener::bind(&socket).unwrap();
        let journal = Arc::new(Journal::default());
        // A prior start is still queued. Only a real pre-start drain can move its processing
        // timestamp before the baseline; selecting the latest UUID alone would reuse it.
        journal.queued.lock().unwrap().push(record(reader::STARTED, OLD, 0));
        let state = Arc::new(Mutex::new(("2026-11-17T10:00:00Z".into(), false)));
        let calls = Arc::new(AtomicUsize::new(0));
        let (source, current, mutations) = (journal.clone(), state.clone(), calls.clone());
        let app = Router::new().fallback(any(move |request: Request| {
            let (source, current, mutations) = (source.clone(), current.clone(), mutations.clone());
            async move {
                if request.uri().path().ends_with("/start") {
                    assert!(request.uri().path().contains(ID), "mutations never use a reusable name");
                    mutations.fetch_add(1, Ordering::SeqCst);
                    if failed {
                        let status = if source.server_failure.load(Ordering::SeqCst) {
                            axum::http::StatusCode::INTERNAL_SERVER_ERROR
                        } else {
                            axum::http::StatusCode::CONFLICT
                        };
                        return (status, json!({"message":"fixture start failed"}).to_string());
                    }
                    // Engine wall time rolls backwards. The monotonic barrier must still bind.
                    if !source.reuse_started.load(Ordering::SeqCst) {
                        *current.lock().unwrap() = ("2026-11-16T10:00:00Z".into(), false);
                    }
                    if !suppressed {
                        source.queued.lock().unwrap().push(record(reader::STARTED, NEW, 0));
                        if multiple {
                            source.queued.lock().unwrap().push(record(reader::STARTED, OLD, 0));
                        }
                    }
                    return (axum::http::StatusCode::NO_CONTENT, String::new());
                }
                if request.uri().path().ends_with("/events") {
                    return (axum::http::StatusCode::OK, String::new());
                }
                let (started, running) = current.lock().unwrap().clone();
                let runtime = if source.runc.load(Ordering::SeqCst) { "runc" } else { "runsc" };
                (
                    axum::http::StatusCode::OK,
                    json!({"Id":ID, "Name":"/owned", "HostConfig":{"Runtime":runtime},
                    "State":{"Status":if running {"running"} else {"exited"}, "OOMKilled":false,
                        "ExitCode":crate::runtime::exit_by(libc::SIGKILL), "StartedAt":started,
                        "FinishedAt":"2026-11-16T10:00:01Z"}})
                    .to_string(),
                )
            }
        }));
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self { directory, socket, journal, state, calls, server }
    }

    fn runtime(&self) -> DockerRuntime {
        let runtime = DockerRuntime::new(&self.socket).unwrap();
        let mut runtime = if self.journal.runc.load(Ordering::SeqCst) { runtime } else { runtime.gvisor() };
        runtime.oom_journal = Some(Context { state: self.directory.path().into(), source: self.journal.clone() });
        runtime
    }

    fn oom(&self, invocation: &str) {
        self.journal.queued.lock().unwrap().push(record(reader::OOM, invocation, 0));
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}

#[tokio::test]
async fn a_fresh_backend_recovers_the_kernel_receipt_after_engine_evidence_is_lost() {
    for runc in [false, true] {
        let fixture = Fixture::new(false, false, false);
        fixture.journal.runc.store(runc, Ordering::SeqCst);
        let runtime = fixture.runtime();
        runtime.start(ID).await.unwrap();
        assert_eq!(fixture.calls.load(Ordering::SeqCst), 1);
        assert!(fixture.journal.syncs.load(Ordering::SeqCst) >= 2);
        // Delayed PID 1 publication can follow FinishedAt. A previous negative observation must
        // not hide it, and there is no Docker OOM flag, event, in-memory binding or realtime join.
        fixture.oom(NEW);
        let fresh = fixture.runtime();
        assert!(fresh.inspect("owned").await.unwrap().unwrap().oom_killed);
        let held = receipt::Held::acquire(fixture.directory.path(), ID).unwrap();
        assert_eq!(held.receipt.bound.as_ref().unwrap().invocation, NEW);
        assert_eq!(held.receipt.bound.as_ref().unwrap().started, "2026-11-16T10:00:00Z");
    }
}

#[tokio::test]
async fn a_suppressed_or_ambiguous_new_start_never_reuses_the_old_invocation() {
    for (suppressed, multiple) in [(true, false), (false, true)] {
        let fixture = Fixture::new(suppressed, false, multiple);
        fixture.oom(OLD);
        fixture.runtime().start("owned").await.unwrap();
        assert_eq!(fixture.calls.load(Ordering::SeqCst), 1);
        assert!(
            !fixture.runtime().inspect("owned").await.unwrap().unwrap().oom_killed,
            "unknown cannot fall back to the old UUID"
        );
        let held = receipt::Held::acquire(fixture.directory.path(), ID).unwrap();
        assert!(held.receipt.bound.is_none());
    }
}

#[tokio::test]
async fn a_failed_start_preserves_the_previous_binding() {
    for server_failure in [false, true] {
        let fixture = Fixture::new(false, true, false);
        fixture.journal.server_failure.store(server_failure, Ordering::SeqCst);
        let previous =
            receipt::Bound { started: "2026-11-17T10:00:00Z".into(), boot: boot().unwrap(), invocation: OLD.into() };
        {
            let mut held = receipt::Held::acquire(fixture.directory.path(), ID).unwrap();
            held.receipt.bound = Some(previous.clone());
            held.save().unwrap();
        }
        fixture.oom(OLD);
        assert!(fixture.runtime().inspect(ID).await.unwrap().unwrap().oom_killed);
        assert!(fixture.runtime().start("owned").await.is_err());
        assert_eq!(fixture.calls.load(Ordering::SeqCst), 1);
        let held = receipt::Held::acquire(fixture.directory.path(), ID).unwrap();
        if server_failure {
            assert!(held.receipt.bound.is_none(), "an actual server failure cannot prove a nonmutating start");
        } else {
            assert_eq!(held.receipt.bound, Some(previous));
        }
        drop(held);
        assert_eq!(
            fixture.runtime().inspect(ID).await.unwrap().unwrap().oom_killed,
            !server_failure,
            "unchanged owner state preserves only a definite conflict, never a generic server rejection"
        );
    }
}

#[tokio::test]
async fn an_unconfirmed_start_is_unknown_after_backend_restart() {
    let fixture = Fixture::new(false, false, false);
    let runtime = fixture.runtime();
    let info = super::super::container::info_from(runtime.docker().unwrap().inspect_container(ID, None).await.unwrap());
    let held = runtime.oom_journal.as_ref().unwrap().begin(&info).await.unwrap();
    drop(held); // crash after persisting intent, before durable generation confirmation
    runtime.docker().unwrap().start_container(ID, None).await.unwrap();
    fixture.oom(NEW);
    assert!(
        !fixture.runtime().inspect(ID).await.unwrap().unwrap().oom_killed,
        "a pending intent cannot be promoted to a confirmed start after the crash gap"
    );
    let held = receipt::Held::acquire(fixture.directory.path(), ID).unwrap();
    assert!(held.receipt.bound.is_none());
    assert!(held.receipt.pending.is_some());
}

#[tokio::test]
async fn a_confirmed_binding_from_another_boot_cannot_supply_evidence() {
    let fixture = Fixture::new(false, false, false);
    fixture.runtime().start(ID).await.unwrap();
    fixture.oom(NEW);
    let mut held = receipt::Held::acquire(fixture.directory.path(), ID).unwrap();
    held.receipt.bound.as_mut().unwrap().boot = "dddddddddddddddddddddddddddddddd".into();
    held.save().unwrap();
    drop(held);
    assert!(!fixture.runtime().inspect(ID).await.unwrap().unwrap().oom_killed);
}

#[tokio::test]
async fn delayed_receipts_are_not_negative_cached_and_sigkill_is_not_oom() {
    let fixture = Fixture::new(false, false, false);
    fixture.runtime().start(ID).await.unwrap();
    assert!(!fixture.runtime().inspect(ID).await.unwrap().unwrap().oom_killed);
    fixture.oom(OLD);
    assert!(!fixture.runtime().inspect(ID).await.unwrap().unwrap().oom_killed);
    fixture.oom(NEW);
    assert!(fixture.runtime().inspect(ID).await.unwrap().unwrap().oom_killed);
    *fixture.state.lock().unwrap() = ("2026-11-18T10:00:00Z".into(), false);
    assert!(
        !fixture.runtime().inspect(ID).await.unwrap().unwrap().oom_killed,
        "later Docker generations cannot inherit it"
    );
}

#[tokio::test]
async fn malformed_journal_reads_and_concurrent_receipt_mutations_fail_closed() {
    let fixture = Fixture::new(false, false, false);
    fixture.journal.malformed.store(true, Ordering::SeqCst);
    fixture.runtime().start(ID).await.unwrap();
    let held = receipt::Held::acquire(fixture.directory.path(), ID).unwrap();
    assert!(matches!(receipt::Held::acquire(fixture.directory.path(), ID), Err(RuntimeError::Conflict(_))));
    drop(held);
    let path = fixture.directory.path().join("docker-oom").join(format!("{ID}.json"));
    std::fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink(fixture.directory.path().join("stolen"), &path).unwrap();
    assert!(receipt::Held::acquire(fixture.directory.path(), ID).is_err());
}

#[tokio::test]
async fn only_fixed_native_manager_authority_can_supply_a_receipt() {
    let fixture = Fixture::new(false, false, false);
    for (field, value) in [
        ("_PID", "2"),
        ("_UID", "1000"),
        ("_TRANSPORT", "stdout"),
        ("UNIT", "docker-neighbor.scope"),
        ("INVOCATION_ID", "truncated"),
        ("_BOOT_ID", "dddddddddddddddddddddddddddddddd"),
    ] {
        let mut value_record = record(reader::OOM, NEW, 1);
        value_record[field] = value.into();
        assert!(
            reader::parse(
                serde_json::to_string(&value_record).unwrap().as_bytes(),
                &boot().unwrap(),
                &format!("docker-{ID}.scope"),
                reader::OOM,
                Some(NEW)
            )
            .is_err()
        );
    }
    let mut duplicate = record(reader::OOM, NEW, 1);
    duplicate["INVOCATION_ID"] = json!([NEW, OLD]);
    assert!(
        reader::parse(
            serde_json::to_string(&duplicate).unwrap().as_bytes(),
            &boot().unwrap(),
            &format!("docker-{ID}.scope"),
            reader::OOM,
            Some(NEW)
        )
        .is_err()
    );
    drop(fixture);
}

#[tokio::test]
async fn an_uncaptured_mutation_retires_an_older_pending_operation() {
    let fixture = Fixture::new(true, false, false);
    let mut held = receipt::Held::acquire(fixture.directory.path(), ID).unwrap();
    held.receipt.pending = Some(receipt::Pending {
        prior_started: Some("2026-11-15T10:00:00Z".into()),
        boot: Some(boot().unwrap()),
        begin: Some(1),
    });
    held.save().unwrap();
    drop(held);
    // The only available start record belongs to B. C's capture is unavailable and its
    // start record is suppressed. An old pending barrier must never bind B's OOM to C.
    fixture.journal.visible.lock().unwrap().push(record(reader::STARTED, OLD, 2));
    fixture.oom(OLD);
    fixture.journal.unavailable.store(true, Ordering::SeqCst);
    fixture.runtime().start(ID).await.unwrap();
    fixture.journal.unavailable.store(false, Ordering::SeqCst);
    assert!(!fixture.runtime().inspect(ID).await.unwrap().unwrap().oom_killed);
    let held = receipt::Held::acquire(fixture.directory.path(), ID).unwrap();
    assert!(held.receipt.pending.is_none());
    assert!(held.receipt.bound.is_none());
}

#[tokio::test]
async fn direct_gvisor_callers_and_unavailable_journals_keep_the_single_start_mutation() {
    for contextual in [false, true] {
        let fixture = Fixture::new(true, false, false);
        fixture.journal.unavailable.store(true, Ordering::SeqCst);
        let mut runtime = fixture.runtime();
        if !contextual {
            runtime.oom_journal = None;
        }
        runtime.start("owned").await.unwrap();
        assert_eq!(fixture.calls.load(Ordering::SeqCst), 1);
        assert!(!runtime.inspect(ID).await.unwrap().unwrap().oom_killed);
    }
}

#[tokio::test]
async fn a_successful_start_with_reused_wall_time_cannot_inherit_the_old_oom() {
    for (unavailable, crash_gap) in [(true, false), (false, false), (true, true), (false, true)] {
        let fixture = Fixture::new(false, false, false);
        let previous =
            receipt::Bound { started: "2026-11-17T10:00:00Z".into(), boot: boot().unwrap(), invocation: OLD.into() };
        {
            let mut held = receipt::Held::acquire(fixture.directory.path(), ID).unwrap();
            held.receipt.bound = Some(previous);
            held.save().unwrap();
        }
        fixture.oom(OLD);
        assert!(fixture.runtime().inspect(ID).await.unwrap().unwrap().oom_killed);
        fixture.journal.unavailable.store(unavailable, Ordering::SeqCst);
        fixture.journal.reuse_started.store(true, Ordering::SeqCst);
        let runtime = fixture.runtime();
        if crash_gap {
            let before = super::super::container::info_from(
                runtime.docker().unwrap().inspect_container(ID, None).await.unwrap(),
            );
            let held = runtime.oom_journal.as_ref().unwrap().begin(&before).await.unwrap();
            assert!(held.receipt.pending.is_some(), "even unavailable capture must durably tombstone the old proof");
            drop(held); // a crash after preparation must not reveal the previous OOM
            assert!(!fixture.runtime().inspect(ID).await.unwrap().unwrap().oom_killed);
        }
        runtime.start(ID).await.unwrap();
        fixture.journal.unavailable.store(false, Ordering::SeqCst);
        assert!(
            !fixture.runtime().inspect(ID).await.unwrap().unwrap().oom_killed,
            "a successful new start with identical StartedAt cannot inherit the previous UUID"
        );
        assert_eq!(fixture.calls.load(Ordering::SeqCst), 1);
        let held = receipt::Held::acquire(fixture.directory.path(), ID).unwrap();
        assert!(held.receipt.bound.is_none() && held.receipt.pending.is_none());
    }
}

#[tokio::test]
async fn a_corrupt_pending_store_cannot_admit_a_mutation_then_recover_stale_evidence() {
    let fixture = Fixture::new(true, false, false);
    let held = receipt::Held::acquire(fixture.directory.path(), ID).unwrap();
    held.save().unwrap();
    drop(held);
    let receipt = fixture.directory.path().join("docker-oom").join(format!("{ID}.json"));
    std::fs::write(receipt, b"{corrupt").unwrap();
    assert!(fixture.runtime().start(ID).await.is_err());
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn receipt_retirement_keeps_active_lock_inodes_and_removes_owned_metadata() {
    let directory = tempfile::tempdir().unwrap();
    let held = receipt::Held::acquire(directory.path(), ID).unwrap();
    held.save().unwrap();
    assert!(receipt::retire(directory.path(), ID).is_err());
    assert!(directory.path().join("docker-oom").join(format!("{ID}.lock")).exists());
    drop(held);
    receipt::retire(directory.path(), ID).unwrap();
    assert_eq!(std::fs::read_dir(directory.path().join("docker-oom")).unwrap().count(), 0);
    let held = receipt::Held::acquire(directory.path(), ID).unwrap();
    assert!(held.receipt.bound.is_none() && held.receipt.pending.is_none());
}

#[test]
fn relative_state_roots_keep_the_existing_configuration_contract() {
    let runtime = DockerRuntime::new(Path::new("/fixture.sock")).unwrap().with_oom_journal(Path::new("./state"));
    assert!(runtime.journal_context().is_some(), "the default Docker backend uses its configured context");
    *runtime.info.lock().unwrap() = Some(crate::runtime::RuntimeInfo { podman: true, ..Default::default() });
    assert!(runtime.journal_context().is_none(), "Podman's native lifecycle keeps its existing path");
    assert_eq!(runtime.oom_journal.unwrap().state, std::env::current_dir().unwrap().join("./state"));
}
