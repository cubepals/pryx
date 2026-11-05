// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! The external debug fixture may use its former executable name; the normal CLI remains pryxd.
#[test]
#[cfg(debug_assertions)]
fn version_alias_is_scoped_to_the_debug_fixture_executable() {
    let bin = env!("CARGO_BIN_EXE_pryxd");
    let normal = std::process::Command::new(bin).arg("--version").output().unwrap();
    assert!(normal.status.success());
    assert_eq!(String::from_utf8_lossy(&normal.stdout), format!("pryxd {}\n", env!("CARGO_PKG_VERSION")));
    let dir = tempfile::tempdir().unwrap();
    let alias = dir.path().join("blocklyd");
    std::fs::copy(bin, &alias).unwrap();
    let legacy = std::process::Command::new(&alias).arg("--version").output().unwrap();
    assert!(legacy.status.success());
    assert_eq!(String::from_utf8_lossy(&legacy.stdout), format!("blocklyd {}\n", env!("CARGO_PKG_VERSION")));
}
