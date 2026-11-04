// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Release download validation, atomic swaps and rollback regression tests.

use super::*;

/// A stand-in pryxd: a script that says it is `version` and does nothing else.
fn script(path: &Path, version: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, format!("#!/bin/sh\n[ \"$1\" = --version ] && echo 'pryxd {version}'\n")).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

fn layout(root: &Path) -> Layout {
    let layout = Layout::new(&root.join("var/lib/pryxd"), root);
    script(&layout.bin, "0.2.0");
    layout
}

fn trial() -> Trial {
    Trial { from: "0.2.0".into(), to: "0.3.0".into(), sha256: "ab".repeat(32) }
}

#[test]
fn a_swap_keeps_the_binary_before_and_puts_the_new_one_on_trial() {
    let root = tempfile::tempdir().unwrap();
    let layout = layout(root.path());
    script(&layout.next(), "0.3.0");
    swap(&layout, &trial()).unwrap();
    assert_eq!(version_of(&layout.bin).unwrap(), "0.3.0");
    assert_eq!(version_of(&layout.prev()).unwrap(), "0.2.0");
    assert!(!layout.next().exists());
    assert_eq!(on_trial(&layout, "0.3.0"), Some(trial()));
    // The binary before it, started instead, drops a trial that isn't its own.
    assert_eq!(on_trial(&layout, "0.2.0"), None);
    assert_eq!(on_trial(&layout, "0.3.0"), None);
}

#[test]
fn a_failed_trial_puts_the_binary_before_back_and_says_why() {
    let root = tempfile::tempdir().unwrap();
    let layout = layout(root.path());
    script(&layout.next(), "0.3.0");
    swap(&layout, &trial()).unwrap();
    assert!(roll_back(&layout, &trial(), "it didn't come up").unwrap());
    assert_eq!(version_of(&layout.bin).unwrap(), "0.2.0");
    assert!(!layout.prev().exists());
    assert_eq!(on_trial(&layout, "0.3.0"), None);
    assert_eq!(failure(&layout), Some(UpgradeFailure { version: "0.3.0".into(), reason: "it didn't come up".into() }));
}

#[test]
fn after_a_stop_only_a_trial_that_exited_badly_is_put_back() {
    let root = tempfile::tempdir().unwrap();
    let layout = layout(root.path());
    // Nothing on trial: whatever the exit, nothing changes.
    assert_eq!(exited(&layout, Some("exit-code"), Some("exited"), Some("1")).unwrap(), "");
    script(&layout.next(), "0.3.0");
    swap(&layout, &trial()).unwrap();
    // The old binary's own clean exit, right after the swap; and a stop (systemctl stop, the
    // host shutting down) before the trial ended, whose SIGTERM systemd counts as clean.
    assert_eq!(exited(&layout, Some("success"), Some("exited"), Some("0")).unwrap(), "");
    assert_eq!(exited(&layout, None, Some("exited"), Some("0")).unwrap(), "");
    assert_eq!(exited(&layout, Some("success"), Some("killed"), Some("TERM")).unwrap(), "");
    assert_eq!(on_trial(&layout, "0.3.0"), Some(trial()), "the trial carries on at the next start");
    let said = exited(&layout, None, Some("exited"), Some("1")).unwrap();
    assert_eq!(said, "pryxd 0.3.0 exited with status 1 before it came up: put pryxd 0.2.0 back\n");
    assert_eq!(version_of(&layout.bin).unwrap(), "0.2.0");
    assert_eq!(failure(&layout).unwrap().reason, "pryxd 0.3.0 exited with status 1 before it came up");
    // Its ExecStartPre failed, so it never ran: systemd says so, with no EXIT_CODE or EXIT_STATUS.
    script(&layout.next(), "0.3.0");
    swap(&layout, &trial()).unwrap();
    let said = exited(&layout, Some("exit-code"), None, None).unwrap();
    assert_eq!(said, "pryxd 0.3.0 didn't start (exit-code) before it came up: put pryxd 0.2.0 back\n");
    assert_eq!(version_of(&layout.bin).unwrap(), "0.2.0");
}

#[test]
fn a_version_is_what_the_binary_prints() {
    let dir = tempfile::tempdir().unwrap();
    let good = dir.path().join("good");
    script(&good, "0.3.0");
    assert_eq!(version_of(&good).unwrap(), "0.3.0");
    let broken = dir.path().join("broken");
    fs::write(&broken, "#!/bin/sh\nexit 1\n").unwrap();
    fs::set_permissions(&broken, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(version_of(&broken).unwrap_err().contains("failed"));
    let chatty = dir.path().join("chatty");
    fs::write(&chatty, "#!/bin/sh\necho hello there\n").unwrap();
    fs::set_permissions(&chatty, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(version_of(&chatty).unwrap_err().contains("not a pryxd version"));
    assert!(version_of(&dir.path().join("missing")).is_err());
}

#[test]
fn a_plain_binary_is_moved_under_the_state_directory_and_linked() {
    let root = tempfile::tempdir().unwrap();
    let layout = Layout::new(&root.path().join("var/lib/pryxd"), root.path());
    script(&layout.link, "0.2.0");
    settle(&layout, None).unwrap();
    assert_eq!(fs::read_link(&layout.link).unwrap(), layout.bin);
    assert_eq!(version_of(&layout.link).unwrap(), "0.2.0");
    // Only the binary the service runs upgrades itself.
    let elsewhere = root.path().join("target/debug/pryxd");
    script(&elsewhere, "0.2.0");
    let refused = settle(&layout, Some(&elsewhere)).unwrap_err();
    assert!(refused.to_string().contains("only the pryxd the service runs"), "{refused}");
    settle(&layout, Some(&layout.link)).unwrap();
    let empty = Layout::new(&root.path().join("elsewhere"), &root.path().join("none"));
    assert!(settle(&empty, None).unwrap_err().to_string().contains("no pryxd"));
}

/// A control plane that serves `binary` at /fleet/v1/blocklyd, over plain HTTP.
async fn serving(binary: Option<Vec<u8>>) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = axum::Router::new().route(
        "/fleet/v1/blocklyd",
        axum::routing::get(move || {
            let binary = binary.clone();
            async move {
                match binary {
                    Some(bytes) => (axum::http::StatusCode::OK, bytes),
                    None => (axum::http::StatusCode::NOT_FOUND, b"no pryxd here".to_vec()),
                }
            }
        }),
    );
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{address}")
}

/// An offer of `version` and `sha256`, signed for them by `key`.
fn signed(key: &rcgen::KeyPair, version: &str, sha256: &str) -> UpgradeOffer {
    UpgradeOffer {
        version: version.into(),
        sha256: sha256.into(),
        signature: Some(trust::test_signature(key, version, sha256)),
    }
}

#[tokio::test]
async fn an_offer_is_installed_only_when_its_sha256_and_version_match() {
    let root = tempfile::tempdir().unwrap();
    let layout = layout(root.path());
    let (key, keys) = trust::test_key();
    let new = root.path().join("new");
    script(&new, "0.3.0");
    let bytes = fs::read(&new).unwrap();
    let sha = hex::encode(Sha256::digest(&bytes));
    let url = serving(Some(bytes)).await;
    let install = |offer: UpgradeOffer| {
        let (layout, url, keys) = (layout.clone(), url.clone(), keys.clone());
        async move { install(&layout, &url, None, &offer, &keys, "0.2.0", None).await }
    };

    let wrong = install(signed(&key, "0.3.0", &"0".repeat(64))).await.unwrap_err();
    assert!(matches!(wrong, UpgradeError::Failed(ref e) if e.contains("not the")), "{wrong}");
    assert!(!layout.next().exists());
    let other = install(signed(&key, "0.4.0", &sha)).await.unwrap_err();
    assert!(other.to_string().contains("not the 0.4.0 offered"), "{other}");
    assert_eq!(version_of(&layout.bin).unwrap(), "0.2.0");
    assert_eq!(on_trial(&layout, "0.3.0"), None);

    assert_eq!(install(signed(&key, "0.3.0", &sha)).await.unwrap(), "0.3.0");
    assert_eq!(version_of(&layout.bin).unwrap(), "0.3.0");
    assert_eq!(version_of(&layout.prev()).unwrap(), "0.2.0");
    assert_eq!(on_trial(&layout, "0.3.0").unwrap(), Trial { from: "0.2.0".into(), to: "0.3.0".into(), sha256: sha });
}

#[tokio::test]
async fn an_offer_without_a_release_signature_for_it_is_never_downloaded_or_run() {
    let root = tempfile::tempdir().unwrap();
    let layout = layout(root.path());
    let (key, keys) = trust::test_key();
    let (stranger, _) = trust::test_key();
    // A binary that leaves a mark if it is ever run, even for its --version.
    let ran = root.path().join("ran");
    let new = root.path().join("new");
    fs::write(&new, format!("#!/bin/sh\ntouch '{}'\necho 'pryxd 0.3.0'\n", ran.display())).unwrap();
    let bytes = fs::read(&new).unwrap();
    let sha = hex::encode(Sha256::digest(&bytes));
    let url = serving(Some(bytes)).await;

    let unsigned = UpgradeOffer { signature: None, ..signed(&key, "0.3.0", &sha) };
    let by_a_stranger = signed(&stranger, "0.3.0", &sha);
    let for_another_version = UpgradeOffer { version: "0.3.0".into(), ..signed(&key, "0.3.1", &sha) };
    let garbled = UpgradeOffer { signature: Some("00".repeat(64)), ..signed(&key, "0.3.0", &sha) };
    // Signed, but older than the 0.2.0 this node runs.
    let older = signed(&key, "0.1.0", &sha);
    for (offer, why) in [
        (unsigned, "carries no release signature"),
        (by_a_stranger, "isn't a pryxd release key's"),
        (for_another_version, "isn't a pryxd release key's"),
        (garbled, "isn't a pryxd release key's"),
        (older, "never goes back"),
    ] {
        let refused = install(&layout, &url, None, &offer, &keys, "0.2.0", None).await.unwrap_err();
        assert!(matches!(refused, UpgradeError::Failed(ref e) if e.contains(why)), "{refused}");
        assert_eq!(version_of(&layout.bin).unwrap(), "0.2.0");
        assert!(!layout.next().exists(), "nothing was downloaded");
        assert!(!layout.prev().exists());
        assert!(!layout.trial().exists(), "no trial.json");
        assert!(!ran.exists(), "the binary offered never ran");
    }
}

#[tokio::test]
async fn a_control_plane_without_a_binary_fails_the_upgrade_and_one_that_is_down_is_tried_again() {
    let root = tempfile::tempdir().unwrap();
    let layout = layout(root.path());
    let (key, keys) = trust::test_key();
    let offer = signed(&key, "0.3.0", &"0".repeat(64));
    let missing = install(&layout, &serving(None).await, None, &offer, &keys, "0.2.0", None).await.unwrap_err();
    assert!(
        matches!(missing, UpgradeError::Failed(ref e) if e.contains(axum::http::StatusCode::NOT_FOUND.as_str())),
        "{missing}"
    );
    let closed = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap().local_addr().unwrap();
    let down = install(&layout, &format!("http://{closed}"), None, &offer, &keys, "0.2.0", None).await;
    assert!(matches!(down, Err(UpgradeError::Transient(_))));
}

#[tokio::test]
async fn a_trial_ends_with_a_heartbeat_once_reconciled_or_puts_the_binary_before_back() {
    let root = tempfile::tempdir().unwrap();
    let state = root.path().join("var/lib/pryxd");
    let layout = layout(root.path());
    let ours = Trial { to: daemon_version(), ..trial() };
    script(&layout.next(), &ours.to);
    swap(&layout, &ours).unwrap();

    let ended = Arc::new(AtomicBool::new(false));
    let upgrader = Arc::new(Upgrader::new(&state, "https://control", ended.clone(), CancellationToken::new()));
    upgrader.accepted(false);
    assert!(on_trial(&layout, &ours.to).is_some());
    assert!(!ended.load(Ordering::SeqCst));
    upgrader.accepted(true);
    assert!(on_trial(&layout, &ours.to).is_none());
    assert!(ended.load(Ordering::SeqCst), "the backstop is disarmed");
    upgrader.clone().watch_trial().await;
    assert_eq!(upgrader.rolled_back(), None);

    script(&layout.bin, "0.2.0");
    script(&layout.next(), &ours.to);
    swap(&layout, &ours).unwrap();
    let cancel = CancellationToken::new();
    let ended = Arc::new(AtomicBool::new(false));
    let mut upgrader = Upgrader::new(&state, "https://control", ended.clone(), cancel.clone());
    upgrader.trial_for = Duration::from_millis(10);
    let upgrader = Arc::new(upgrader);
    upgrader.clone().watch_trial().await;
    assert!(cancel.is_cancelled());
    assert!(!ended.load(Ordering::SeqCst), "the backstop still bounds the stop");
    assert!(upgrader.rolled_back().unwrap().contains("didn't reconcile"));
    assert_eq!(version_of(&layout.bin).unwrap(), "0.2.0");
    assert_eq!(upgrader.failure().unwrap().version, ours.to);
}
