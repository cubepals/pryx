// SPDX-License-Identifier: FSL-1.1-ALv2
//! Durable execute-only agent copies for private exec completion. Linux and gVisor make an
//! unreadable executable non-dumpable while loading its image, before a sibling can open proc
//! descriptors. A late prctl alone cannot revoke an already opened stdin descriptor.
//! Runtime registration and exec I/O stay in their owning modules.
use super::RuntimeError;
use sha2::{Digest, Sha256};
use std::{
    fs::{File, Permissions},
    io::{Read, Write},
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
};

pub(super) fn stage(binary: &Path, data: &Path) -> Result<PathBuf, RuntimeError> {
    staged(binary, data).map_err(|error| {
        RuntimeError::Other(
            if error.kind() == std::io::ErrorKind::Unsupported {
                "gvisor needs a static release agent; a GNU or dynamic development build cannot protect exec completion"
            } else {
                "exec-timeout needs a protected execute-only agent copy"
            }
            .into(),
        )
    })
}
fn staged(binary: &Path, data: &Path) -> std::io::Result<PathBuf> {
    // The configured workload store may live below a different host user's directory. It
    // cannot hold a trusted executable, even if this particular file is execute-only. The
    // installed agent's directory is an alternative only with the same ancestor checks.
    let parent = [data.parent(), binary.parent()]
        .into_iter()
        .flatten()
        .find(|parent| protected_directory(parent))
        .ok_or_else(|| std::io::Error::other("no protected directory for the exec agent"))?;
    let mut source = File::open(binary)?;
    let mut staged = tempfile::NamedTempFile::new_in(parent)?;
    let mut digest = Sha256::new();
    let mut bytes = [0; 64 * 1024];
    loop {
        let read = source.read(&mut bytes)?;
        if read == 0 {
            break;
        }
        digest.update(&bytes[..read]);
        staged.write_all(&bytes[..read])?;
    }
    // The image controls its own loader and libraries. Private stdin is delivered only to a
    // self-contained agent, never an image-supplied ELF interpreter or preload library.
    let image = std::fs::read(staged.path())?;
    use object::read::elf::ProgramHeader;
    let elf =
        object::read::elf::ElfFile64::<object::Endianness>::parse(image.as_slice()).map_err(std::io::Error::other)?;
    if elf.elf_program_headers().iter().any(|header| header.p_type(elf.endian()) == object::elf::PT_INTERP) {
        return Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "exec owner must be a static ELF"));
    }
    drop(image);
    staged.as_file().set_permissions(Permissions::from_mode(0o111))?;
    staged.as_file().sync_all()?;
    let target = parent.join(format!("exec-agent-{}", hex::encode(digest.finalize())));
    // Retire the writer before publication; no process may exec a still-writable inode.
    let staged = staged.into_temp_path();
    match staged.persist_noclobber(&target) {
        Ok(_) => (),
        Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => (),
        Err(error) => return Err(error.error),
    }
    if !protected(&target) {
        return Err(std::io::Error::other("unprotected exec agent"));
    }
    File::open(parent)?.sync_all()?;
    Ok(target)
}

pub(super) fn protected(path: &Path) -> bool {
    let own = rustix::process::geteuid().as_raw();
    let Ok(file) = std::fs::symlink_metadata(path) else {
        return false;
    };
    let named = path
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_prefix("exec-agent-"))
        .is_some_and(|digest| digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit()));
    named
        && file.is_file()
        && file.nlink() == 1
        && file.mode() & 0o7777 == 0o111
        && (file.uid() == own || file.uid() == 0)
        && path.parent().is_some_and(protected_directory)
}

fn protected_directory(path: &Path) -> bool {
    let own = rustix::process::geteuid().as_raw();
    path.ancestors().all(|parent| {
        std::fs::symlink_metadata(parent).is_ok_and(|directory| {
            directory.is_dir()
                && (directory.mode() & 0o022 == 0 || (directory.uid() == 0 && directory.mode() & 0o1000 != 0))
                && (directory.uid() == own || directory.uid() == 0)
        })
    })
}

/// Always set the owner label, including zero: absence would inherit an image's forged claim.
pub(super) async fn negotiate(binary: &Path, labels: &mut std::collections::BTreeMap<String, String>) {
    let version = crate::runtime::agent_metadata::query(binary)
        .await
        .map_or(0, |capabilities| capabilities.exec_completion_version);
    labels.insert(crate::labels::LABEL_EXEC_COMPLETION.into(), version.to_string());
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn image_labels_cannot_enable_a_legacy_agents_completion_protocol() {
        let dir = tempfile::tempdir().unwrap();
        let agent = dir.path().join("agent");
        std::os::unix::fs::symlink(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/agent-metadata"), &agent)
            .unwrap();
        std::fs::write(agent.with_extension("json"), "{}").unwrap();
        let mut spec = super::super::tests::spec();
        spec.labels.insert(crate::labels::LABEL_EXEC_COMPLETION.into(), "1".into());
        negotiate(&agent, &mut spec.labels).await;
        let body = super::super::container::body_of(&spec);
        assert_eq!(body.labels.unwrap().get(crate::labels::LABEL_EXEC_COMPLETION).map(String::as_str), Some("0"));
    }

    #[test]
    fn exec_copies_are_static_execute_only_and_never_replaced_while_reused() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("agent.c");
        std::fs::write(&source, "int main(void) { return 0; }\n").unwrap();
        let compile = |name: &str, static_binary: bool| {
            let binary = dir.path().join(name);
            let mut cc = std::process::Command::new("cc");
            if static_binary {
                cc.arg("-static");
            }
            let result = cc.arg(&source).arg("-o").arg(&binary).output().unwrap();
            assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
            binary
        };
        let static_binary = compile("static-agent", true);
        let data = dir.path().join("data");
        std::fs::create_dir(&data).unwrap();
        let copied = stage(&static_binary, &data).unwrap();
        let before = std::fs::symlink_metadata(&copied).unwrap();
        assert!(protected(&copied));
        assert_eq!(before.mode() & 0o7777, 0o111);
        assert_eq!(stage(&static_binary, &data).unwrap(), copied);
        assert_eq!(std::fs::symlink_metadata(&copied).unwrap().ino(), before.ino());
        let dynamic = compile("dynamic-agent", false);
        assert!(stage(&dynamic, &data).unwrap_err().to_string().contains("static release agent"));
    }

    #[test]
    fn an_untrusted_workload_parent_uses_only_a_protected_installation() {
        let installation = tempfile::tempdir().unwrap();
        let source = installation.path().join("agent.c");
        let binary = installation.path().join("agent");
        std::fs::write(&source, "int main(void) { return 0; }\n").unwrap();
        assert!(
            std::process::Command::new("cc")
                .args(["-static"])
                .arg(&source)
                .arg("-o")
                .arg(&binary)
                .status()
                .unwrap()
                .success()
        );
        let store = tempfile::tempdir().unwrap();
        let data = store.path().join("workload");
        std::fs::create_dir(&data).unwrap();
        std::fs::set_permissions(store.path(), Permissions::from_mode(0o777)).unwrap();
        let protected_copy = stage(&binary, &data).unwrap();
        assert_eq!(protected_copy.parent(), Some(installation.path()));
        assert!(protected(&protected_copy));
        std::fs::set_permissions(installation.path(), Permissions::from_mode(0o777)).unwrap();
        assert!(!protected(&protected_copy));
        assert!(stage(&binary, &data).unwrap_err().to_string().contains("protected execute-only agent"));
    }
}
