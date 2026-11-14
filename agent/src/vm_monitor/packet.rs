// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2
//! Validate and prepare the packet helper's inherited private listener and exact filesystem grant.
//! The maintained helper owns networking; this module never implements a packet transport.
use super::Plan;
use std::{fs, os::unix::net::UnixListener, path::Path};

pub(super) fn listener(
    plan: &Plan,
    socket: &Path,
    descriptor_argument: &str,
) -> Result<UnixListener, Box<dyn std::error::Error>> {
    if socket.parent() != Some(plan.directory.as_path())
        || !plan.write.is_empty()
        || descriptor_argument
            .strip_prefix("--")
            .is_none_or(|name| name.is_empty() || !name.bytes().all(|byte| byte.is_ascii_lowercase() || byte == b'-'))
        || plan.arguments.iter().any(|arg| arg.split('=').next() == Some(descriptor_argument))
    {
        return Err("Packet proxy requires its own private listener and inherited descriptor".into());
    }
    let listener = UnixListener::bind(socket)?;
    fs::set_permissions(socket, std::os::unix::fs::PermissionsExt::from_mode(0o600))?;
    rustix::io::fcntl_setfd(&listener, rustix::io::FdFlags::empty())?;
    super::confine(plan, false)?;
    Ok(listener)
}
