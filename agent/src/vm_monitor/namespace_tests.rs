// SPDX-License-Identifier: FSL-1.1-ALv2
//! Reject changed kernel identities and ordinary files before any namespace transition.
use super::*;

#[test]
fn changed_namespace_inode_is_refused_before_setns() {
    let file = fs::File::open("/proc/self/ns/net").unwrap();
    let identity = file.metadata().unwrap();
    let namespace = Namespace {
        descriptor: file.as_raw_fd(),
        device: identity.dev(),
        inode: identity.ino().checked_add(1).unwrap(),
    };
    let before = fs::read_link("/proc/self/ns/net").unwrap();
    assert_eq!(enter_namespace(&namespace, std::process::id()).unwrap_err().kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(fs::read_link("/proc/self/ns/net").unwrap(), before);
}

#[test]
fn matching_regular_file_identity_does_not_grant_a_namespace() {
    let file = tempfile::tempfile().unwrap();
    let identity = file.metadata().unwrap();
    let namespace = Namespace { descriptor: file.as_raw_fd(), device: identity.dev(), inode: identity.ino() };
    let before = fs::read_link("/proc/self/ns/net").unwrap();
    assert_eq!(enter_namespace(&namespace, std::process::id()).unwrap_err().kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(fs::read_link("/proc/self/ns/net").unwrap(), before);
}
