// SPDX-License-Identifier: FSL-1.1-ALv2
//! Real PTY input and retained process metadata across a complete Tokio runtime shutdown.
//! A separate owned test subprocess bounds a blocking-thread regression without an engine.

use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::{Engine, engine::general_purpose::STANDARD};
use pryx_agent::process::{Command as ProcessCommand, Event, Process, ProcessError, Processes, Start};

const RETAINED: &str = "real PTY finished with registry and process retained";

#[test]
fn completed_pty_input_does_not_hold_retained_registry_runtime_shutdown() {
    const FIXTURE: &str = "PRYX_TEST_PTY_RUNTIME_RETIREMENT";
    const NAME: &str = "completed_pty_input_does_not_hold_retained_registry_runtime_shutdown";
    if std::env::var_os(FIXTURE).is_some() {
        retained_registry();
        return;
    }
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", NAME, "--nocapture", "--test-threads", "1"])
        .env(FIXTURE, "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let until = Instant::now() + Duration::from_secs(10);
    while child.try_wait().unwrap().is_none() && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(10));
    }
    let finished = child.try_wait().unwrap().is_some();
    if !finished {
        // Only the direct fixture is killed; its real PTY child has already completed before
        // the retained-registry runtime drop this regression is designed to bound.
        child.kill().unwrap();
    }
    let output = child.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(RETAINED), "the fixture did not finish its real PTY before runtime shutdown");
    assert!(finished, "a retained completed PTY kept the runtime from retiring");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(stdout.contains("retained PTY runtime retired"));
}

fn retained_registry() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
    let processes = Arc::new(Processes::new(directory.path().into(), "UNUSED_PTY_FIXTURE_BOOTSTRAP".into()));
    let (process, input_closed) = runtime.block_on(async {
        let process = processes
            .start(
                Start {
                    command: ProcessCommand::Shell(
                        "printf 'ready\\n'; read first; printf 'first:%s\\n' \"$first\"; \
                         read second; printf 'second:%s\\n' \"$second\""
                            .into(),
                    ),
                    env: Default::default(),
                    cwd: None,
                    stdin: true,
                    pty: true,
                    cols: Some(80),
                    rows: Some(24),
                    timeout: Some(5),
                },
                "retained-pty",
                Some(5),
            )
            .unwrap();
        output_contains(&process, "ready").await;
        // Each observer is dropped by output_contains. A live terminal still accepts input
        // without an attached observer and can replay it to a later attachment.
        assert!(process.view().running);
        process.write(b"one\n").await.unwrap();
        output_contains(&process, "first:one").await;
        assert!(process.view().running);
        process.write(b"two\n").await.unwrap();
        output_contains(&process, "second:two").await;
        let completed = tokio::time::timeout(Duration::from_secs(5), process.wait()).await.unwrap();
        assert_eq!(completed.code, Some(0));
        let input_closed = matches!(process.write(b"").await, Err(ProcessError::Invalid("stdin is closed")));
        (process, input_closed)
    });
    processes.close();
    assert!(Arc::ptr_eq(&processes.get(&process.id).unwrap(), &process));
    assert!(!process.view().running);
    println!("{RETAINED}");
    std::io::stdout().flush().unwrap();
    // This is the executable's critical ownership order: completed metadata and its process
    // registry remain alive while all runtime workers, including blocking PTY I/O, must end.
    drop(runtime);
    assert!(input_closed, "stdin is closed before completed metadata is published");
    assert!(Arc::ptr_eq(&processes.get(&process.id).unwrap(), &process));
    assert!(!process.view().running);
    println!("retained PTY runtime retired");
}

async fn output_contains(process: &Arc<Process>, expected: &str) {
    tokio::time::timeout(Duration::from_secs(5), async {
        let (replay, mut live) = process.subscribe().await;
        let mut events = replay.into_iter();
        let mut output = String::new();
        loop {
            let event = match events.next() {
                Some(event) => event,
                None => live.recv().await.expect("the live PTY has output"),
            };
            if let Event::Output { bytes, .. } = event {
                output.push_str(&String::from_utf8(STANDARD.decode(bytes).unwrap()).unwrap());
                if output.contains(expected) {
                    break;
                }
            }
        }
    })
    .await
    .expect("the real PTY reaches its expected phase");
}
