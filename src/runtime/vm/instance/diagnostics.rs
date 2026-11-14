// SPDX-License-Identifier: FSL-1.1-ALv2
//! Test-only retention of bounded monitor tails in CI artifacts. Private journals, bootstrap
//! input and environment values are never exported. This is not a production logging switch.
use super::*;
use std::io::{Read, Seek, SeekFrom, Write};

pub(super) fn retain(serial: &Path, secrets: &[String]) {
    let Some(output) = std::env::var_os("PRYX_VM_RUNTIME_DIAGNOSTICS") else { return };
    if let Err(problem) = retain_to(Path::new(&output), serial, secrets) {
        eprintln!("VM runtime diagnostic retention failed: {problem}");
    }
}
pub(super) async fn failed(monitor: &mut Machine, serial: &Path, spec: &ContainerSpec) {
    // A startup refusal never authorizes another boot. Retire the owned child and
    // drain its final error before the temporary launch directory can disappear.
    if monitor.exited().is_ok_and(|status| status.is_none()) {
        let _ = tokio::time::timeout(crate::runtime::ENGINE_PROBE_TIMEOUT, monitor.kill()).await;
    }
    let _ = tokio::time::timeout(crate::runtime::ENGINE_PROBE_TIMEOUT, monitor.finish_serial()).await;
    retain(serial, &spec.env.iter().map(|(_, value)| value.clone()).collect::<Vec<_>>());
}
fn retain_to(output: &Path, serial: &Path, secrets: &[String]) -> Result<PathBuf, RuntimeError> {
    directory(output)?;
    let mut file =
        OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(serial).map_err(error)?;
    let info = file.metadata().map_err(error)?;
    if !info.is_file()
        || info.uid() != rustix::process::geteuid().as_raw()
        || info.nlink() != 1
        || info.mode() & 0o077 != 0
    {
        return Err(error("Unsafe runtime diagnostic source"));
    }
    let bound = pryx_agent::contract::MAX_REQUEST_BYTES as u64;
    let start = info.len().saturating_sub(bound);
    file.seek(SeekFrom::Start(start)).map_err(error)?;
    let mut bytes = Vec::new();
    file.take(bound.min(info.len())).read_to_end(&mut bytes).map_err(error)?;
    // Discard a truncated line before redaction, including any partial credential at its edge.
    let bytes = if start == 0 {
        bytes.as_slice()
    } else {
        bytes.iter().position(|byte| *byte == b'\n').map_or(&[][..], |line| &bytes[line + 1..])
    };
    // A running monitor may still be writing its last line. Never retain a partial
    // value from that edge either.
    let bytes = bytes.iter().rposition(|byte| *byte == b'\n').map_or(&[][..], |last| &bytes[..=last]);
    let mut redacted = String::from_utf8_lossy(bytes).into_owned();
    for value in secrets.iter().filter(|value| !value.is_empty()) {
        redacted = redacted.replace(value, "[redacted]");
    }
    let path = output.join(format!("{}.serial.log", uuid::Uuid::new_v4().simple()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&path)
        .map_err(error)?;
    file.write_all(redacted.as_bytes()).map_err(error)?;
    Ok(path)
}
#[test]
fn retained_tails_redact_values_refuse_links_and_bound_artifact_bytes() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let serial = root.path().join("serial");
    let output = root.path().join("evidence");
    let secret = uuid::Uuid::new_v4().simple().to_string();
    let mut bytes = vec![b'x'; pryx_agent::contract::MAX_REQUEST_BYTES];
    bytes.extend_from_slice(format!("\n{secret}\nfinal\n").as_bytes());
    crate::durable::write_atomic(&serial, &bytes, 0o600).unwrap();
    let path = retain_to(&output, &serial, std::slice::from_ref(&secret)).unwrap();
    let retained = fs::read_to_string(path).unwrap();
    assert_eq!(retained, "[redacted]\nfinal\n");
    assert!(!retained.contains(&secret));
    assert!(retained.len() <= pryx_agent::contract::MAX_REQUEST_BYTES);
    let link = root.path().join("link");
    symlink(&serial, &link).unwrap();
    assert!(retain_to(&output, &link, &[]).is_err());
    fs::hard_link(&serial, root.path().join("another")).unwrap();
    assert!(retain_to(&output, &serial, &[]).is_err());
}
