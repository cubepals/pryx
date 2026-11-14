// SPDX-License-Identifier: FSL-1.1-ALv2
//! Actual root bind-mount, inode and ownership handoff tests, without claiming a VMM boot.
//! The optional VM workflow runs the ignored root case. Ordinary hosts check refusal before
//! filesystem mutation; privileged acceptance remains separate from normal user tests.
use super::*;

#[test]
fn ordinary_user_is_refused_before_root_jail_mutation() {
    assert!(!rustix::process::geteuid().is_root());
    let root = tempfile::tempdir().unwrap();
    let base = root.path().join("absent");
    let id = uuid::Uuid::new_v4().simple().to_string();
    let result = Jail::prepare(Request {
        base: &base,
        id: &id,
        monitor: Path::new("firecracker.v1.18.0"),
        disk: &root.path().join(disk::FILE),
        lease: &root.path().join("lease"),
        owner: (rustix::process::geteuid().as_raw(), rustix::process::getegid().as_raw()),
        readonly: &BTreeMap::new(),
    });
    assert!(matches!(result, Err(RuntimeError::Unavailable(_))));
    assert!(!base.exists());
}

#[test]
#[ignore = "requires real root mount/ownership authority; optional VM CI runs this exact test"]
fn actual_root_mounts_retire_after_leases_and_recover_without_a_second_disk() {
    assert!(rustix::process::geteuid().is_root());
    let owner = (
        std::env::var("PRYX_VM_ROOT_TEST_UID").unwrap().parse::<u32>().unwrap(),
        std::env::var("PRYX_VM_ROOT_TEST_GID").unwrap().parse::<u32>().unwrap(),
    );
    let root = tempfile::tempdir().unwrap();
    let base = root.path().join("jails");
    let data = root.path().join("data");
    let locks = root.path().join("locks");
    for path in [&base, &data, &locks] {
        fs::DirBuilder::new().mode(0o700).create(path).unwrap();
    }
    let disk = data.join(disk::FILE);
    crate::durable::write_atomic(&disk, b"original", 0o600).unwrap();
    let original = fs::metadata(&disk).unwrap();
    let source = root.path().join("kernel");
    crate::durable::write_atomic(&source, b"read-only", 0o444).unwrap();
    let inputs = BTreeMap::from([(PathBuf::from("kernel"), source.clone())]);
    let lifetime = locks.join("monitor.lock");
    let id = uuid::Uuid::new_v4().simple().to_string();
    let mut jail = Jail::prepare(Request {
        base: &base,
        id: &id,
        monitor: Path::new("firecracker.v1.18.0"),
        disk: &disk,
        lease: &lifetime,
        owner,
        readonly: &inputs,
    })
    .unwrap();
    assert_eq!(jail.root(), base.join("firecracker.v1.18.0").join(&id).join(ROOT));
    assert_eq!(fs::metadata(&disk).unwrap().ino(), original.ino());
    assert_eq!(fs::metadata(&disk).unwrap().uid(), owner.0);
    assert_eq!(fs::metadata(jail.root().join(disk::FILE)).unwrap().ino(), original.ino());
    let configuration = root.path().join("configuration");
    crate::durable::write_atomic(&configuration, b"generated", 0o444).unwrap();
    let held = lease(&lifetime).unwrap();
    assert!(matches!(jail.add_readonly(Path::new("configuration"), &configuration), Err(RuntimeError::Conflict(_))));
    assert!(matches!(jail.retire(), Err(RuntimeError::Conflict(_))));
    assert_eq!(fs::metadata(&disk).unwrap().uid(), owner.0);
    drop(held);
    jail.add_readonly(Path::new("configuration"), &configuration).unwrap();
    assert_eq!(
        fs::metadata(jail.root().join("configuration")).unwrap().ino(),
        fs::metadata(&configuration).unwrap().ino()
    );
    // Match the jailer's documented final root-directory ownership, then exercise real DAC
    // and mount enforcement from an ordinary chrooted process. No guest/VMM is simulated.
    std::os::unix::fs::chown(jail.root(), Some(owner.0), Some(owner.1)).unwrap();
    let child = std::process::Command::new("python3")
        .args(["-c", "import errno,os,sys; os.chroot(sys.argv[1]); os.chdir('/'); os.setgroups([]); os.setgid(int(sys.argv[3])); os.setuid(int(sys.argv[2])); f=open(sys.argv[4],'r+b'); f.write(b'changed!'); f.close(); assert open('/kernel','rb').read()==b'read-only'; assert open('/configuration','rb').read()==b'generated'\ntry: open('/kernel','wb')\nexcept OSError as error: assert error.errno in (errno.EACCES,errno.EROFS)\nelse: raise AssertionError('Read-only jail input was writable')"])
        .arg(jail.root()).arg(owner.0.to_string()).arg(owner.1.to_string()).arg(format!("/{}", disk::FILE)).output().unwrap();
    assert!(child.status.success(), "Root handoff child: {}", String::from_utf8_lossy(&child.stderr));
    assert!(jail.add_readonly(Path::new("late-configuration"), &configuration).is_err());
    jail.retire().unwrap();
    jail.retire().unwrap();
    assert!(!jail.root().exists());
    assert_eq!(fs::metadata(&disk).unwrap().uid(), rustix::process::Uid::ROOT.as_raw());
    assert_eq!(fs::metadata(&disk).unwrap().ino(), original.ino());
    assert_eq!(fs::read(&disk).unwrap(), b"changed!");
    assert_eq!(fs::read(&source).unwrap(), b"read-only");
    recover_after_tampering(&base, &disk, &source, &lifetime, owner, &inputs, &original);
}

fn recover_after_tampering(
    base: &Path,
    disk: &Path,
    source: &Path,
    lifetime: &Path,
    owner: (u32, u32),
    inputs: &BTreeMap<PathBuf, PathBuf>,
    original: &fs::Metadata,
) {
    let id = uuid::Uuid::new_v4().simple().to_string();
    let jail = Jail::prepare(Request {
        base,
        id: &id,
        monitor: Path::new("firecracker.v1.18.0"),
        disk,
        lease: lifetime,
        owner,
        readonly: inputs,
    })
    .unwrap();
    let path = jail.root().to_owned();
    drop(jail);
    assert_eq!(fs::metadata(disk).unwrap().uid(), owner.0);
    let retained = disk.parent().unwrap().join("retained");
    fs::rename(disk, &retained).unwrap();
    std::os::unix::fs::symlink(source, disk).unwrap();
    assert!(Jail::recover(base, &id).is_err());
    assert_eq!(fs::read(source).unwrap(), b"read-only");
    assert_eq!(fs::metadata(&retained).unwrap().uid(), owner.0);
    fs::remove_file(disk).unwrap();
    fs::rename(retained, disk).unwrap();
    Jail::recover(base, &id).unwrap();
    assert!(!path.exists());
    assert_eq!(fs::metadata(disk).unwrap().uid(), rustix::process::Uid::ROOT.as_raw());
    assert_eq!(fs::metadata(disk).unwrap().ino(), original.ino());
    assert_eq!(fs::metadata(disk).unwrap().nlink(), 1);
}
