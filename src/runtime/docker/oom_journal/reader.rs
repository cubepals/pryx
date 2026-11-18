// SPDX-License-Identifier: FSL-1.1-ALv2
//! Fixed systemd records from the native host journal, never message text. The maintained
//! journalctl synchronizer drains queued input before acknowledging the causal barrier.
use super::{RuntimeError, failure, valid_hex};
use async_trait::async_trait;
use rustix::fs::{Mode, OFlags};
use std::{
    fs::{File, Metadata, OpenOptions},
    os::{
        fd::AsRawFd,
        unix::fs::{MetadataExt, OpenOptionsExt},
    },
    path::{Component, Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};
use tokio::io::AsyncReadExt;

// systemd v255 src/systemd/sd-messages.h, UNIT_STARTED and UNIT_OUT_OF_MEMORY.
pub(super) const STARTED: &str = "39f53479d3a045ac8e11786248231fbf";
pub(super) const OOM: &str = "fe6faa94e7774663a0da52717891d8ef";
const FIELDS: &str = "MESSAGE_ID,UNIT,INVOCATION_ID,_BOOT_ID,_PID,_UID,_TRANSPORT,__MONOTONIC_TIMESTAMP";
const BYTES: u64 = 16 * 1024;
const BUDGET: Duration = Duration::from_secs(3);

#[async_trait]
pub(super) trait Source: Send + Sync {
    async fn run(&self, args: &[String]) -> Result<Vec<u8>, RuntimeError>;
    async fn available(&self) -> Result<(), RuntimeError>;
}

pub(super) struct Native;

// systemd v255 sd-journal.c opens system.journal and system@*.journal[~] in each
// default-namespace machine directory. Bound the metadata walk and inherited authority set.
const ENTRIES: usize = 128;
const FILES: usize = 64;
const STORES: [&str; 2] = ["/run/log/journal", "/var/log/journal"];

fn anchored(file: &File) -> PathBuf {
    PathBuf::from(format!("/proc/{}/fd/{}", std::process::id(), file.as_raw_fd()))
}

fn owned(metadata: &Metadata, owner: u32) -> bool {
    metadata.uid() == owner && metadata.mode() & 0o022 == 0
}

fn directory(path: &Path, owner: u32) -> Result<Option<File>, RuntimeError> {
    // Never follow an input link, even in an ancestor. Group-writable /var/log is
    // legitimate: authority comes from the protected leaf and pinned journal files.
    if !path.is_absolute() {
        return Err(failure("journal directory must be absolute"));
    }
    let mut held = File::open("/").map_err(|_| failure("cannot open journal root"))?;
    for part in path.components() {
        let Component::Normal(part) = part else {
            if part == Component::RootDir {
                continue;
            }
            return Err(failure("journal directory is not an absolute normal path"));
        };
        let next = match rustix::fs::openat(
            &held,
            part,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        ) {
            Ok(fd) => File::from(fd),
            Err(rustix::io::Errno::NOENT) => return Ok(None),
            Err(_) => return Err(failure("cannot open an unlinked journal directory")),
        };
        let metadata = next.metadata().map_err(|_| failure("cannot inspect journal directory"))?;
        if !metadata.is_dir() || (metadata.uid() != owner && metadata.uid() != 0) {
            return Err(failure("journal directory has another owner"));
        }
        held = next;
    }
    let metadata = held.metadata().map_err(|_| failure("cannot inspect journal directory"))?;
    if !owned(&metadata, owner) {
        return Err(failure("journal directory is writable by another owner"));
    }
    Ok(Some(held))
}

fn journal_name(name: &str) -> bool {
    (name == "system.journal" || name == "system.journal~")
        || (name.starts_with("system@") && (name.ends_with(".journal") || name.ends_with(".journal~")))
}

fn files_in(directory: &File, owner: u32, files: &mut Vec<File>, entries: &mut usize) -> Result<(), RuntimeError> {
    let path = anchored(directory);
    for entry in std::fs::read_dir(&path).map_err(|_| failure("cannot enumerate system journals"))? {
        *entries += 1;
        if *entries > ENTRIES {
            return Err(failure("journal metadata walk exceeded its bound"));
        }
        let entry = entry.map_err(|_| failure("cannot inspect system journal entry"))?;
        if !entry.file_name().to_str().is_some_and(journal_name) {
            continue;
        }
        if files.len() == FILES {
            return Err(failure("system journal file set exceeded its bound"));
        }
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
            .open(entry.path())
            .map_err(|_| failure("cannot open an unlinked system journal"))?;
        let metadata = file.metadata().map_err(|_| failure("cannot inspect system journal file"))?;
        if !metadata.is_file() || !owned(&metadata, owner) || metadata.nlink() != 1 {
            return Err(failure("system journal file is not protected"));
        }
        files.push(file);
    }
    Ok(())
}

pub(super) fn journal_files(paths: &[&Path], owner: u32) -> Result<Vec<File>, RuntimeError> {
    let mut files = Vec::new();
    let mut entries = 0;
    for path in paths {
        let Some(root) = directory(path, owner)? else { continue };
        files_in(&root, owner, &mut files, &mut entries)?;
        for entry in std::fs::read_dir(anchored(&root)).map_err(|_| failure("cannot enumerate journal machines"))? {
            entries += 1;
            if entries > ENTRIES {
                return Err(failure("journal metadata walk exceeded its bound"));
            }
            let entry = entry.map_err(|_| failure("cannot inspect journal machine entry"))?;
            if !entry.file_name().to_str().is_some_and(|name| valid_hex(name, 32)) {
                continue;
            }
            // proc's intentional FD anchor pins the already-open parent. The directory
            // entry itself is opened relative to that FD without following any link.
            let child = rustix::fs::openat(
                &root,
                entry.file_name(),
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map(File::from)
            .map_err(|_| failure("cannot open an unlinked journal machine"))?;
            let metadata = child.metadata().map_err(|_| failure("cannot inspect journal machine"))?;
            if !metadata.is_dir() || !owned(&metadata, owner) {
                return Err(failure("journal machine is not protected"));
            }
            files_in(&child, owner, &mut files, &mut entries)?;
        }
    }
    if files.is_empty() {
        return Err(failure("no protected system journal files are available"));
    }
    Ok(files)
}

fn binary() -> Result<std::path::PathBuf, RuntimeError> {
    if !rustix::process::geteuid().is_root() {
        return Err(failure("requires the native root system journal"));
    }
    let init = std::fs::read_link("/proc/1/exe").map_err(|_| failure("cannot authenticate host PID 1"))?;
    if init.file_name().is_none_or(|name| name != "systemd") || crate::runtime::protected_agent(&init, true).is_none() {
        return Err(failure("requires native systemd PID 1"));
    }
    // A container or journal namespace must not substitute its own trusted-field authority.
    let ours = std::fs::metadata("/proc/self/ns/pid").map_err(|_| failure("cannot inspect PID namespace"))?;
    let host = std::fs::metadata("/proc/1/ns/pid").map_err(|_| failure("cannot inspect PID 1 namespace"))?;
    if ours.dev() != host.dev() || ours.ino() != host.ino() {
        return Err(failure("requires the system manager PID namespace"));
    }
    ["/usr/bin/journalctl", "/bin/journalctl"]
        .iter()
        .find_map(|path| crate::runtime::protected_agent(Path::new(path), true))
        .ok_or_else(|| failure("requires a protected journalctl executable"))
}

async fn native_version() -> Result<std::path::PathBuf, RuntimeError> {
    let binary = binary()?;
    let output = execute(&binary, &["--version".into()]).await?;
    let version = std::str::from_utf8(&output)
        .ok()
        .and_then(|text| text.lines().next())
        .and_then(|line| line.strip_prefix("systemd "))
        .and_then(|line| line.split_whitespace().next())
        .and_then(|version| version.parse::<u32>().ok());
    if version.is_none_or(|version| version < 255) {
        return Err(failure("requires journalctl 255 or newer with the native Synchronize method"));
    }
    Ok(binary)
}

#[async_trait]
impl Source for Native {
    async fn available(&self) -> Result<(), RuntimeError> {
        let binary = native_version().await?;
        let _files = journal_files(&STORES.map(Path::new), 0)?;
        execute(&binary, &["--system".into(), "--sync".into()]).await.map(|_| ())
    }

    async fn run(&self, args: &[String]) -> Result<Vec<u8>, RuntimeError> {
        let binary = native_version().await?;
        if args.iter().any(|arg| arg == "--sync") {
            return execute(&binary, args).await;
        }
        let files = journal_files(&STORES.map(Path::new), 0)?;
        let mut selected = args.iter().filter(|arg| arg.as_str() != "--system").cloned().collect::<Vec<_>>();
        // v255 journalctl's repeated --file options feed sd_journal_open_files, which
        // merges this exact FD set and disables discovery of any additional files.
        selected.extend(files.iter().map(|file| format!("--file={}", anchored(file).display())));
        execute(&binary, &selected).await
    }
}

async fn execute(binary: &Path, args: &[String]) -> Result<Vec<u8>, RuntimeError> {
    let mut child = tokio::process::Command::new(binary)
        .args(args)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| failure("cannot start journalctl"))?;
    let mut stdout = child.stdout.take().expect("piped journal stdout").take(BYTES + 1);
    let mut stderr = child.stderr.take().expect("piped journal stderr").take(BYTES + 1);
    let read = async {
        let mut output = Vec::new();
        let mut errors = Vec::new();
        let (out, err) = tokio::join!(stdout.read_to_end(&mut output), stderr.read_to_end(&mut errors));
        out.map_err(|_| failure("cannot read journalctl output"))?;
        err.map_err(|_| failure("cannot read journalctl status"))?;
        if output.len() as u64 > BYTES || errors.len() as u64 > BYTES {
            return Err(failure("journalctl output exceeded its bound"));
        }
        let status = child.wait().await.map_err(|_| failure("cannot reap journalctl"))?;
        if !status.success() || !errors.is_empty() {
            return Err(failure("journalctl did not complete a clean bounded read"));
        }
        Ok(output)
    };
    match tokio::time::timeout(BUDGET, read).await {
        Ok(Ok(output)) => Ok(output),
        result => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            match result {
                Ok(Err(error)) => Err(error),
                _ => Err(failure("journalctl exceeded its deadline")),
            }
        }
    }
}

pub(super) async fn sync(source: &Arc<dyn Source>) -> Result<(), RuntimeError> {
    // v255 journalctl.c action_sync uses io.systemd.Journal.Synchronize. journald's
    // deferred +15 request drains all queued input (+5), syncs, then replies.
    let output = source.run(&["--system".into(), "--sync".into()]).await?;
    if !output.is_empty() {
        return Err(failure("unexpected journal synchronization output"));
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Record {
    pub(super) invocation: String,
    pub(super) monotonic: u64,
}

pub(super) async fn query(
    source: &Arc<dyn Source>,
    boot: &str,
    id: &str,
    message: &str,
    invocation: Option<&str>,
) -> Result<Vec<Record>, RuntimeError> {
    let scope = format!("docker-{id}.scope");
    let mut args = vec![
        "--system".into(),
        "--no-pager".into(),
        "--quiet".into(),
        "--output=json".into(),
        format!("--output-fields={FIELDS}"),
        "--all".into(),
        "--lines=2".into(),
        format!("_BOOT_ID={boot}"),
        "_PID=1".into(),
        "_UID=0".into(),
        "_TRANSPORT=journal".into(),
        format!("UNIT={scope}"),
        format!("MESSAGE_ID={message}"),
    ];
    if let Some(invocation) = invocation {
        args.push(format!("INVOCATION_ID={invocation}"));
    }
    let output = source.run(&args).await?;
    parse(&output, boot, &scope, message, invocation)
}

pub(super) fn parse(
    output: &[u8],
    boot: &str,
    scope: &str,
    message: &str,
    invocation: Option<&str>,
) -> Result<Vec<Record>, RuntimeError> {
    let mut records = Vec::new();
    for line in output.split(|byte| *byte == b'\n').filter(|line| !line.is_empty()) {
        if records.len() == 2 {
            return Err(failure("journal query returned too many records"));
        }
        let value: serde_json::Value = serde_json::from_slice(line).map_err(|_| failure("malformed journal record"))?;
        let field = |name: &str| value.get(name).and_then(serde_json::Value::as_str);
        if field("MESSAGE_ID") != Some(message)
            || field("UNIT") != Some(scope)
            || field("_BOOT_ID") != Some(boot)
            || field("_PID") != Some("1")
            || field("_UID") != Some("0")
            || field("_TRANSPORT") != Some("journal")
        {
            return Err(failure("journal record authority or scope changed"));
        }
        let id = field("INVOCATION_ID")
            .filter(|id| valid_hex(id, 32))
            .ok_or_else(|| failure("journal invocation identity is unavailable"))?;
        if invocation.is_some_and(|expected| expected != id) {
            return Err(failure("journal invocation identity changed"));
        }
        let monotonic = field("__MONOTONIC_TIMESTAMP")
            .and_then(|value| value.parse().ok())
            .ok_or_else(|| failure("journal monotonic timestamp is unavailable"))?;
        records.push(Record { invocation: id.into(), monotonic });
    }
    Ok(records)
}
