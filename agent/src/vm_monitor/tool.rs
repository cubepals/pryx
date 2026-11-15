// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2
//! Confinement for one host filesystem program (mke2fs, e2fsck, mkfs.erofs) over a disk a guest
//! wrote or layers a tenant chose. It keeps the launcher's exact Landlock grants (its staged
//! inputs, its one output and its private directory), creates no socket, holds no capability
//! even as root, dumps no core and writes no file past its bound. It never receives a device,
//! a namespace or the monitor's KVM access; which program and files it gets is pryxd's choice.
use rustix::process::{Resource, Rlimit};
use std::path::Path;

/// Descriptors a filesystem program needs: its input, its output, the loader and libraries.
const OPEN_FILES: u64 = 256;

pub(super) fn confine(
    plan: &super::Plan,
    file_bytes: u64,
    configuration: Option<&Path>,
    command: &mut std::process::Command,
) -> Result<(), Box<dyn std::error::Error>> {
    if file_bytes == 0 {
        return Err("A filesystem program needs a positive written-file bound".into());
    }
    if let Some(configuration) = configuration {
        if !plan.read.iter().any(|path| configuration.starts_with(path)) {
            return Err("A filesystem program's configuration must be one of its read grants".into());
        }
        super::regular(configuration)?;
        // e2fsprogs reads its profile from this variable (mke2fs.conf(5)); nothing else is passed.
        command.env("MKE2FS_CONFIG", configuration);
    }
    // Its temporary files (erofs-utils and e2fsprogs honour TMPDIR) stay in its private directory.
    command.env("TMPDIR", &plan.directory);
    for (resource, value) in [(Resource::Core, 0), (Resource::Fsize, file_bytes), (Resource::Nofile, OPEN_FILES)] {
        rustix::process::setrlimit(resource, Rlimit { current: Some(value), maximum: Some(value) })?;
    }
    // Dropped before the syscall policy, which refuses capability changes. Landlock's
    // restriction then sets no_new_privs, so the program's exec cannot regain them: root keeps
    // only its identity, for the root-owned staged copy.
    rustix::thread::set_capabilities(
        None,
        rustix::thread::CapabilitySets {
            effective: rustix::thread::CapabilitySet::empty(),
            permitted: rustix::thread::CapabilitySet::empty(),
            inheritable: rustix::thread::CapabilitySet::empty(),
        },
    )?;
    super::confine(plan, false)
}
