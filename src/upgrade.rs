// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

//! pryxd upgrading itself to the pryxd its control plane serves (`GET /fleet/v1/blocklyd`,
//! docs/fleet-operations.md §9): when a heartbeat's answer offers it, which the control plane does
//! for one node of a region at a time, or at once with `pryxd upgrade` (`cli/upgrade.rs`).
//!
//! ```text
//! <state_dir>/bin/pryxd          the binary systemd runs: /usr/local/bin/pryxd links here
//! <state_dir>/bin/pryxd.prev     the one before it, kept to go back to
//! <state_dir>/upgrade/trial.json    an upgrade that hasn't come up yet: from, to, sha256
//! <state_dir>/upgrade/failed.json   the last upgrade given up on, which heartbeats report
//! ```
//!
//! The binary lives under the state directory because the unit's `ProtectSystem=strict` leaves
//! pryxd nothing else to write, and the unit needs no new writable path for it. A
//! `ReadWritePaths=` naming /usr/local/bin/pryxd would bind-mount that one file, which a rename
//! can't replace, and naming /usr/local/bin would open every binary there.
//!
//! 1. An offer is taken only if a pryxd release key signed its version and sha256, and its
//!    version isn't older than this one (`trust.rs`). The binary is then downloaded over the node's
//!    mutual TLS to `pryxd.next`, and kept only if its sha256 is the one offered and it runs
//!    here, saying it is the version offered.
//! 2. The current binary is kept as `pryxd.prev`, the trial is written down, and
//!    `pryxd.next` is renamed over `pryxd`: one rename, so a crash leaves one or the other.
//! 3. pryxd exits cleanly and systemd (`Restart=always`) starts the new one. Sandboxes keep
//!    running: they are Docker's, not pryxd's children.
//! 4. The new one is on trial while `trial.json` names its version: it must reconcile and have a
//!    heartbeat accepted within TRIAL_SECONDS, which ends the trial. Otherwise it puts
//!    `pryxd.prev` back and exits. One that can't even start is put back by the unit's
//!    `ExecStopPost`, `pryxd.prev upgrade --exited`, after a stop systemd doesn't call a
//!    success, its `ExecStartPre` failing included. One that hangs is ended by its backstop.
//! 5. The pryxd put back says why in its heartbeats, until the next upgrade is tried.
//!
//! Parts (`upgrade/`):
//! - `tests.rs`: release validation, swap and rollback regression tests.
//! - `trust.rs`: whether an offer is taken: a release's signature, and never an older version.
//! - `stopped.rs`: what `--exited` goes by: systemd's word on the stop, and the state directory.
//! - `backstop.rs`: ends a start still on trial long after it began, for `--exited` to put back.

pub mod backstop;
pub mod stopped;
pub(crate) mod trust;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Context;
use http_body_util::BodyExt;
use hyper::Method;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::durable::write_atomic;
use crate::http_client as client;
use crate::protocol::wire::FLEET_API;
use crate::protocol::wire::{UpgradeFailure, UpgradeOffer};
use crate::upgrade::trust::ReleaseKeys;

/// How long a new pryxd has to reconcile and get a heartbeat accepted before it is put back.
pub(crate) const TRIAL_SECONDS: u64 = 120;
/// No pryxd is this big; a download that is stops.
const MAX_BYTES: u64 = 512 * 1024 * 1024;
const DOWNLOAD_TIMEOUT: Duration = Duration::from_mins(10);
/// How long the control plane has to start answering for its pryxd or the pryxd's sha256.
pub(crate) const ANSWER_TIMEOUT: Duration = Duration::from_secs(30);
/// After a failed attempt, an offer is taken up again only this long after: by then the control
/// plane has heard of a failure, and offers again only once an operator retries.
const RETRY_AFTER: Duration = Duration::from_mins(1);
const LINK: &str = "/usr/local/bin/pryxd";

/// The version this pryxd reports to the control plane, which offers an upgrade to an older one.
/// A debug build reports PRYXD_TEST_VERSION instead when it is set, so an end-to-end test can
/// run a node that looks older than the pryxd it is offered.
pub fn daemon_version() -> String {
    #[cfg(debug_assertions)]
    if let Some(version) = std::env::var("PRYXD_TEST_VERSION")
        .or_else(|_| std::env::var("BLOCKLYD_TEST_VERSION"))
        .ok()
        .filter(|v| !v.is_empty())
    {
        return version;
    }
    env!("CARGO_PKG_VERSION").to_owned()
}

/// An unchanged external debug contract installs the test binary under the former name.
/// Production builds always use pryxd, regardless of their executable name.
pub fn debug_legacy_executable() -> bool {
    #[cfg(debug_assertions)]
    {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.file_name().map(|n| n == "blocklyd" || n == "blocklyd.prev"))
            .unwrap_or(false)
    }
    #[cfg(not(debug_assertions))]
    {
        false
    }
}

/// Where the binaries and the upgrade's own state are.
#[derive(Clone, Debug)]
pub struct Layout {
    pub(crate) bin: PathBuf,
    /// The path the unit runs, which links to `bin`.
    pub(crate) link: PathBuf,
    dir: PathBuf,
}

impl Layout {
    /// The layout for `state_dir`, with the link under `root` (`/` but in tests).
    pub fn new(state_dir: &Path, root: &Path) -> Self {
        Self {
            bin: state_dir.join("bin").join(if debug_legacy_executable() { "blocklyd" } else { "pryxd" }),
            link: root.join(LINK.trim_start_matches('/')),
            dir: state_dir.join("upgrade"),
        }
    }

    pub(crate) fn next(&self) -> PathBuf {
        self.bin.with_extension("next")
    }

    pub(crate) fn prev(&self) -> PathBuf {
        self.bin.with_extension("prev")
    }

    fn trial(&self) -> PathBuf {
        self.dir.join("trial.json")
    }

    fn failed(&self) -> PathBuf {
        self.dir.join("failed.json")
    }
}

/// An upgrade that hasn't come up yet.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Trial {
    pub from: String,
    pub to: String,
    pub sha256: String,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum UpgradeError {
    /// Worth trying again: the control plane didn't answer, or not all of it.
    #[error("{0}")]
    Transient(String),
    /// Told to the control plane, which stops the region's rollout.
    #[error("{0}")]
    Failed(String),
}

fn failed(what: &str, path: &Path) -> impl FnOnce(std::io::Error) -> UpgradeError {
    let what = format!("{what} {}", path.display());
    move |e| UpgradeError::Failed(format!("{what}: {e}"))
}

/// Steps 1 and 2: the binary the control plane serves, for an offer `keys` vouch for to a node
/// that runs `from`, checked against the offer's sha256 and version, put in place with the current
/// one kept and the trial written down. `running` is the binary this process runs from, which must
/// be the one it replaces. Returns the version installed.
pub(crate) async fn install(
    layout: &Layout,
    control_plane: &str,
    tls: Option<Arc<rustls::ClientConfig>>,
    offer: &UpgradeOffer,
    keys: &ReleaseKeys,
    from: &str,
    running: Option<&Path>,
) -> Result<String, UpgradeError> {
    let _ = fs::remove_file(layout.failed());
    trust::check(offer, from, keys).map_err(UpgradeError::Failed)?;
    settle(layout, running)?;
    download(layout, control_plane, tls, &offer.sha256).await?;
    let next = layout.next();
    let printed = tokio::task::spawn_blocking(move || version_of(&next))
        .await
        .map_err(|e| UpgradeError::Failed(e.to_string()))?
        .map_err(|e| UpgradeError::Failed(format!("the pryxd downloaded doesn't run here: {e}")))?;
    if printed != offer.version {
        return Err(UpgradeError::Failed(format!(
            "the pryxd downloaded says it is {printed}, not the {} offered",
            offer.version
        )));
    }
    let trial = Trial { from: from.to_owned(), to: printed.clone(), sha256: offer.sha256.clone() };
    swap(layout, &trial)?;
    Ok(printed)
}

/// The binary at `bin`, where an upgrade can replace it. A host that has /usr/local/bin/pryxd as
/// a plain file, from before pryxd upgraded itself, has it moved there and linked, which only
/// works outside the unit's sandbox: as root, from `pryxd upgrade`.
fn settle(layout: &Layout, running: Option<&Path>) -> Result<(), UpgradeError> {
    let canonical = |path: &Path| fs::canonicalize(path).ok();
    if let Some(exe) = running
        && canonical(exe) != canonical(&layout.bin)
        && canonical(exe) != canonical(&layout.link)
    {
        return Err(UpgradeError::Failed(format!(
            "this pryxd runs from {}, not {}: only the pryxd the service runs upgrades itself",
            exe.display(),
            layout.link.display()
        )));
    }
    if layout.bin.is_file() {
        return Ok(());
    }
    if !fs::symlink_metadata(&layout.link).is_ok_and(|m| m.file_type().is_file()) {
        return Err(UpgradeError::Failed(format!("there is no pryxd at {} to upgrade", layout.bin.display())));
    }
    let moved = || -> std::io::Result<()> {
        // The link first, where it can't be made the rest isn't either.
        let link = layout.link.with_file_name("pryxd.link");
        let _ = fs::remove_file(&link);
        std::os::unix::fs::symlink(&layout.bin, &link)?;
        fs::create_dir_all(layout.bin.parent().unwrap_or(Path::new("/")))?;
        fs::copy(&layout.link, layout.next())?;
        fs::set_permissions(layout.next(), fs::Permissions::from_mode(0o755))?;
        fs::rename(layout.next(), &layout.bin)?;
        fs::rename(&link, &layout.link)
    };
    moved().map_err(|e| {
        UpgradeError::Failed(format!(
            "pryxd is a plain file at {}, which can't be moved to {} from here ({e}): run `sudo pryxd upgrade` \
             on the host once",
            layout.link.display(),
            layout.bin.display()
        ))
    })
}

/// `pryxd.next`, from the control plane, if its sha256 is `sha256`.
async fn download(
    layout: &Layout,
    control_plane: &str,
    tls: Option<Arc<rustls::ClientConfig>>,
    sha256: &str,
) -> Result<(), UpgradeError> {
    let url = format!("{}{FLEET_API}/blocklyd", control_plane.trim_end_matches('/'));
    let next = layout.next();
    let transient = |e: &dyn std::fmt::Display| UpgradeError::Transient(format!("{url}: {e}"));
    let fetch = async {
        let response = client::send(Method::GET, &url, &[], client::empty(), tls, ANSWER_TIMEOUT)
            .await
            .map_err(|e| transient(&e))?;
        let status = response.status();
        if !status.is_success() {
            let said = client::read_body(response, 4096).await.unwrap_or_default();
            let said = format!("HTTP {status}: {}", String::from_utf8_lossy(&said).trim());
            return Err(if status.is_server_error() { transient(&said) } else { UpgradeError::Failed(said) });
        }
        let mut file = tokio::fs::File::create(&next).await.map_err(failed("writing", &next))?;
        let (mut hasher, mut size) = (Sha256::new(), 0u64);
        let mut body = response.into_body();
        while let Some(frame) = body.frame().await {
            let Ok(data) = frame.map_err(|e| transient(&e))?.into_data() else { continue };
            size += data.len() as u64;
            if size > MAX_BYTES {
                return Err(UpgradeError::Failed(format!("{url}: more than {MAX_BYTES} bytes, which no pryxd is")));
            }
            hasher.update(&data);
            file.write_all(&data).await.map_err(failed("writing", &next))?;
        }
        file.sync_all().await.map_err(failed("writing", &next))?;
        Ok(hex::encode(hasher.finalize()))
    };
    let got = tokio::time::timeout(DOWNLOAD_TIMEOUT, fetch)
        .await
        .map_err(|_| transient(&format!("not downloaded within {DOWNLOAD_TIMEOUT:?}")))?;
    let got = got.inspect_err(|_| {
        let _ = fs::remove_file(&next);
    })?;
    if !got.eq_ignore_ascii_case(sha256) {
        let _ = fs::remove_file(&next);
        return Err(UpgradeError::Failed(format!(
            "the pryxd downloaded has sha256 {got}, not the {sha256} offered, so it wasn't installed"
        )));
    }
    fs::set_permissions(&next, fs::Permissions::from_mode(0o755)).map_err(failed("making executable", &next))
}

/// What `<bin> --version` says it is: `pryxd 0.3.0` is 0.3.0.
pub(crate) fn version_of(bin: &Path) -> Result<String, String> {
    use std::process::{Command, Stdio};
    let spawn =
        || Command::new(bin).arg("--version").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn();
    // A file just written may still be open in a child another thread forked: try again then.
    let mut child = spawn();
    for _ in 0..5 {
        match &child {
            Err(e) if rustix::io::Errno::from_io_error(e) == Some(rustix::io::Errno::TXTBSY) => {
                std::thread::sleep(Duration::from_millis(50))
            }
            _ => break,
        }
        child = spawn();
    }
    let mut child = child.map_err(|e| e.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(10);
    while child.try_wait().map_err(|e| e.to_string())?.is_none() {
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err("--version didn't finish within 10 s".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&output.stdout);
    let parsed = text
        .trim()
        .strip_prefix("pryxd ")
        .or_else(|| debug_legacy_executable().then(|| text.trim().strip_prefix("blocklyd ")).flatten());
    match parsed {
        Some(version) if output.status.success() && !version.contains(char::is_whitespace) => Ok(version.to_owned()),
        _ if !output.status.success() => Err(format!("--version failed ({})", output.status)),
        _ => Err(format!("--version printed {:?}, not a pryxd version", text.trim())),
    }
}

/// Step 2. The trial is written before the rename: a crash between them leaves the old binary with
/// a trial for another version, which it drops (`on_trial`), rather than an untried new one.
fn swap(layout: &Layout, trial: &Trial) -> Result<(), UpgradeError> {
    let (prev, kept) = (layout.prev(), layout.bin.with_file_name("pryxd.prev.tmp"));
    let _ = fs::remove_file(&kept);
    fs::hard_link(&layout.bin, &kept).map_err(failed("keeping", &layout.bin))?;
    fs::rename(&kept, &prev).map_err(failed("keeping", &prev))?;
    fs::create_dir_all(&layout.dir).map_err(failed("making", &layout.dir))?;
    fs::set_permissions(&layout.dir, fs::Permissions::from_mode(0o700)).map_err(failed("making", &layout.dir))?;
    let text = serde_json::to_vec_pretty(trial).map_err(|e| UpgradeError::Failed(e.to_string()))?;
    write_atomic(&layout.trial(), &text, 0o600).map_err(|e| UpgradeError::Failed(e.to_string()))?;
    fs::rename(layout.next(), &layout.bin).map_err(failed("installing", &layout.bin))?;
    let dir = layout.bin.parent().unwrap_or(Path::new("/"));
    fs::File::open(dir).and_then(|d| d.sync_all()).map_err(failed("syncing", dir))
}

/// The trial this binary, `version`, is on. A trial for another version is left from an upgrade
/// that never ran, or was put back, and is dropped.
pub(crate) fn on_trial(layout: &Layout, version: &str) -> Option<Trial> {
    let text = fs::read(layout.trial()).ok()?;
    match serde_json::from_slice::<Trial>(&text) {
        Ok(trial) if trial.to == version => Some(trial),
        _ => {
            let _ = fs::remove_file(layout.trial());
            None
        }
    }
}

/// The trial passed: the new binary stays, and `pryxd.prev` with it, for going back by hand.
pub(crate) fn confirm(layout: &Layout) {
    let _ = fs::remove_file(layout.trial());
}

/// Puts `pryxd.prev` back for a trial that failed, and writes down why, for the heartbeats.
/// Returns whether there was one to put back.
pub(crate) fn roll_back(layout: &Layout, trial: &Trial, reason: &str) -> anyhow::Result<bool> {
    let prev = layout.prev();
    let restored = prev.is_file();
    if restored {
        fs::rename(&prev, &layout.bin).with_context(|| format!("putting {} back", prev.display()))?;
    }
    record_failure(layout, &trial.to, reason)?;
    confirm(layout);
    Ok(restored)
}

fn record_failure(layout: &Layout, version: &str, reason: &str) -> anyhow::Result<()> {
    fs::create_dir_all(&layout.dir).with_context(|| format!("making {}", layout.dir.display()))?;
    let failure = UpgradeFailure { version: version.to_owned(), reason: reason.to_owned() };
    write_atomic(&layout.failed(), &serde_json::to_vec_pretty(&failure)?, 0o600)?;
    Ok(())
}

/// The last upgrade given up on, which every heartbeat reports until the next one is tried.
pub(crate) fn failure(layout: &Layout) -> Option<UpgradeFailure> {
    serde_json::from_slice(&fs::read(layout.failed()).ok()?).ok()
}

/// `pryxd.prev upgrade --exited`, the unit's `ExecStopPost`, after any stop: a binary on trial
/// that stopped other than cleanly is put back. `result`, `code` and `status` are systemd's
/// `SERVICE_RESULT`, `EXIT_CODE` and `EXIT_STATUS`, which `stopped::failure` reads.
pub fn exited(
    layout: &Layout,
    result: Option<&str>,
    code: Option<&str>,
    status: Option<&str>,
) -> anyhow::Result<String> {
    let Some(trial) = fs::read(layout.trial()).ok().and_then(|t| serde_json::from_slice::<Trial>(&t).ok()) else {
        return Ok(String::new());
    };
    let Some(how) = stopped::failure(result, code, status) else {
        return Ok(String::new());
    };
    let name = if debug_legacy_executable() { "blocklyd" } else { "pryxd" };
    let reason = format!("{name} {} {how} before it came up", trial.to);
    let said = match roll_back(layout, &trial, &reason)? {
        true => format!("{reason}: put {name} {} back\n", trial.from),
        false => format!("{reason}, and there is no {name} before it to put back\n"),
    };
    Ok(said)
}

/// The daemon's side: offers in heartbeat answers, and the trial a start after an upgrade is on.
pub struct Upgrader {
    layout: Layout,
    control_plane: String,
    version: String,
    trial: Mutex<Option<Trial>>,
    keys: ReleaseKeys,
    confirmed: tokio::sync::Notify,
    busy: AtomicBool,
    retry_at: Mutex<Option<Instant>>,
    rolled_back: Mutex<Option<String>>,
    trial_for: Duration,
    /// Set when a heartbeat ends the trial, which disarms the start's backstop (backstop.rs).
    trial_ended: Arc<AtomicBool>,
    /// Stops the daemon: after an upgrade, so systemd starts the new binary.
    cancel: CancellationToken,
}

impl Upgrader {
    pub fn new(state_dir: &Path, control_plane: &str, trial_ended: Arc<AtomicBool>, cancel: CancellationToken) -> Self {
        let layout = Layout::new(state_dir, Path::new("/"));
        let version = daemon_version();
        let trial = on_trial(&layout, &version);
        if let Some(trial) = &trial {
            info!(
                from = %trial.from,
                to = %trial.to,
                seconds = TRIAL_SECONDS,
                "upgraded: on trial until reconciled and a heartbeat is accepted"
            );
        }
        Self {
            layout,
            control_plane: control_plane.to_owned(),
            version,
            trial: Mutex::new(trial),
            keys: ReleaseKeys::ours(),
            confirmed: tokio::sync::Notify::new(),
            busy: AtomicBool::new(false),
            retry_at: Mutex::new(None),
            rolled_back: Mutex::new(None),
            trial_for: Duration::from_secs(TRIAL_SECONDS),
            trial_ended,
            cancel,
        }
    }

    /// What heartbeats report: the last upgrade given up on.
    pub(crate) fn failure(&self) -> Option<UpgradeFailure> {
        failure(&self.layout)
    }

    /// The control plane accepted a heartbeat. One that said the node had reconciled ends a trial.
    pub(crate) fn accepted(&self, reconciled: bool) {
        if !reconciled {
            return;
        }
        let Some(trial) = self.trial.lock().unwrap().take() else { return };
        confirm(&self.layout);
        self.trial_ended.store(true, Ordering::SeqCst);
        info!(from = %trial.from, to = %trial.to, "the upgrade came up: reconciled, and the control plane hears it");
        self.confirmed.notify_one();
    }

    /// Why this start put the previous binary back and stopped, if it did.
    pub fn rolled_back(&self) -> Option<String> {
        self.rolled_back.lock().unwrap().clone()
    }

    /// Waits out a trial. One that doesn't pass in time puts the previous binary back and stops
    /// the daemon, so systemd starts that one.
    pub async fn watch_trial(self: Arc<Self>) {
        if self.trial.lock().unwrap().is_none() {
            return;
        }
        tokio::select! {
            () = self.confirmed.notified() => return,
            () = self.cancel.cancelled() => return,
            () = tokio::time::sleep(self.trial_for) => {}
        }
        let Some(trial) = self.trial.lock().unwrap().take() else { return };
        let reason =
            format!("pryxd {} didn't reconcile and get a heartbeat accepted within {TRIAL_SECONDS} s", trial.to);
        match roll_back(&self.layout, &trial, &reason) {
            Ok(true) => {
                error!(from = %trial.from, to = %trial.to, "{reason}: put {} back; stopping to run it", trial.from);
                *self.rolled_back.lock().unwrap() = Some(format!("{reason}; put {} back", trial.from));
                self.cancel.cancel();
            }
            Ok(false) => warn!(to = %trial.to, "{reason}, and there is no pryxd before it to put back"),
            Err(e) => error!(error = format!("{e:#}"), "{reason}, and putting the one before back failed"),
        }
    }

    /// A heartbeat's answer offered `offer`: it is installed in the background, and pryxd
    /// stops once it is, unless an attempt runs or failed a moment ago.
    pub(crate) fn offered(self: &Arc<Self>, offer: UpgradeOffer, tls: Arc<rustls::ClientConfig>) {
        if offer.version == self.version || self.trial.lock().unwrap().is_some() || self.cancel.is_cancelled() {
            return;
        }
        if self.retry_at.lock().unwrap().is_some_and(|at| Instant::now() < at) || self.busy.swap(true, Ordering::SeqCst)
        {
            return;
        }
        let this = self.clone();
        tokio::spawn(async move {
            info!(from = %this.version, to = %offer.version, "upgrading pryxd, as the control plane offers");
            let running = std::env::current_exe().ok();
            let (layout, keys) = (&this.layout, &this.keys);
            let installed =
                install(layout, &this.control_plane, Some(tls), &offer, keys, &this.version, running.as_deref());
            match installed.await {
                Ok(to) => {
                    info!(from = %this.version, to = %to, "installed; restarting into it, sandboxes keep running");
                    this.cancel.cancel();
                    return;
                }
                Err(UpgradeError::Transient(e)) => {
                    warn!(error = %e, "couldn't download the pryxd offered; trying again in a minute")
                }
                Err(UpgradeError::Failed(e)) => {
                    warn!(error = %e, to = %offer.version, "gave up on the upgrade offered; the control plane is told");
                    if let Err(e) = record_failure(&this.layout, &offer.version, &e) {
                        warn!(error = format!("{e:#}"), "couldn't write the failure down");
                    }
                }
            }
            *this.retry_at.lock().unwrap() = Some(Instant::now() + RETRY_AFTER);
            this.busy.store(false, Ordering::SeqCst);
        });
    }
}

#[cfg(test)]
mod tests;
