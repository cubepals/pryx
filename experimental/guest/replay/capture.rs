// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Replay one stabilized, real guest RAM image with the exact page writer and byte verifier
//! from the registered fixture. This supplemental test writes aggregate observations only.
use super::{Backend, Method, Store, host, new_file, verify, write_pages, zero};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::Instant;

const PAGE: usize = 4096;
const GUEST_BYTES: usize = 256 << 20;

struct OutputFile {
    file: File,
    path: PathBuf,
}
impl Drop for OutputFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

pub(super) fn private_input(path: &Path) -> io::Result<Vec<u8>> {
    let mut f = OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW).open(path)?;
    let meta = f.metadata()?;
    if !meta.is_file()
        || meta.mode() & 0o077 != 0
        || meta.uid() != unsafe { libc::geteuid() }
        || meta.len() != GUEST_BYTES as u64
    {
        return Err(io::Error::other("expected a private ordinary-user 256 MiB guest capture"));
    }
    let mut bytes = Vec::with_capacity(GUEST_BYTES);
    (&mut f).take(GUEST_BYTES as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() != GUEST_BYTES {
        return Err(io::Error::other("captured guest memory changed size"));
    }
    Ok(bytes)
}

fn replay(bytes: &[u8], scratch: &Path, rounds: usize) -> io::Result<String> {
    let meta = fs::symlink_metadata(scratch)?;
    if !meta.is_dir() || meta.mode() & 0o077 != 0 || meta.uid() != unsafe { libc::geteuid() } {
        return Err(io::Error::other("expected a private ordinary-user scratch directory"));
    }
    let candidates = Backend::available();
    let scan = candidates.iter().rev().copied().find(|b| b.name().starts_with("asm-")).unwrap_or(Backend::SCALAR);
    let fingerprint =
        candidates.iter().copied().find(|b| matches!(b.name(), "asm-avx2" | "asm-neon")).unwrap_or(Backend::SCALAR);
    let cases = [
        (Method::Dense, Backend::LIBRARY),
        (Method::Fill, Backend::LIBRARY),
        (Method::Fill, scan),
        (Method::Hash, Backend::LIBRARY),
        (Method::Hash, fingerprint),
    ];
    let zeros = bytes.chunks_exact(PAGE).filter(|chunk| zero(Backend::SCALAR, chunk)).count();
    let mut csv = String::from(
        "round,arch,source_bytes,page_bytes,zero_pages,total_pages,method,backend,elapsed_ns,allocated_bytes,verified\n",
    );

    for trial in 0..=rounds {
        for index in 0..cases.len() {
            let (method, backend) = cases[if trial % 2 == 0 { index } else { cases.len() - 1 - index }];
            let path = scratch.join("guest-page-output");
            let mut out = OutputFile { file: new_file(&path)?, path };
            let start = Instant::now();
            write_pages(method, backend, &mut out.file, bytes, PAGE, Store::Native, 0)?;
            out.file.set_len(bytes.len() as u64)?;
            out.file.sync_data()?;
            let ns = start.elapsed().as_nanos();
            verify(&mut out.file, bytes)?;
            let allocated = out.file.metadata()?.blocks() * 512;
            if trial != 0 {
                let arch = std::env::consts::ARCH;
                let name = backend.name();
                let size = bytes.len();
                let pages = size / PAGE;
                csv.push_str(&format!(
                    "{trial},{arch},{size},{PAGE},{zeros},{pages},{method:?},{name},{ns},{allocated},true\n"
                ));
            }
        }
    }
    Ok(csv)
}

#[test]
fn captured_image() -> io::Result<()> {
    host::pin(0);
    let path = |name| std::env::var_os(name).map(PathBuf::from).ok_or_else(|| io::Error::other("missing replay path"));
    let source = path("PRYX_LAB_CAPTURE_SOURCE")?;
    let scratch = path("PRYX_LAB_CAPTURE_SCRATCH")?;
    let report = path("PRYX_LAB_CAPTURE_REPORT")?;
    if report
        .parent()
        .ok_or_else(|| io::Error::other("missing report parent"))?
        .canonicalize()?
        .starts_with(scratch.canonicalize()?)
    {
        return Err(io::Error::other("aggregate report must be outside the private scratch directory"));
    }
    let bytes = private_input(&source)?;
    let csv = replay(&bytes, &scratch, 15)?;
    let staged = report.with_extension(format!("aggregate-{}.tmp", std::process::id()));
    let mut output =
        OutputFile { file: OpenOptions::new().write(true).create_new(true).mode(0o600).open(&staged)?, path: staged };
    output.file.write_all(csv.as_bytes())?;
    output.file.sync_data()?;
    fs::hard_link(&output.path, report)
}

#[test]
fn protected_small_fixture() -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    let scratch = std::env::temp_dir().join(format!("pryx-replay-smoke-{}", std::process::id()));
    fs::DirBuilder::new().mode(0o700).create(&scratch)?;
    let outcome = (|| -> io::Result<()> {
        let mut bytes = vec![0; 3 * PAGE];
        bytes[PAGE + 7] = 11;
        let csv = replay(&bytes, &scratch, 0)?;
        assert_eq!(csv.lines().count(), 1);
        assert_eq!(fs::read_dir(&scratch)?.count(), 0);
        let input = scratch.join("wrong-size");
        let _ = new_file(&input)?;
        assert!(private_input(&input).is_err());
        Ok(())
    })();
    fs::remove_dir_all(scratch)?;
    outcome
}
