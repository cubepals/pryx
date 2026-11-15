// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! The arguments the node gives the unmodified bubblewrap that starts a process supervisor in
//! namespaces of its own (pryxd's runtime/process/namespaces.rs): here so the agent's own
//! acceptance starts its tools exactly as a node would. Not for the browser's nested helper,
//! whose arguments are its own (process_tools/browser.rs).
use std::ffi::OsString;
use std::path::Path;

/// bubblewrap's arguments for one supervisor, up to and including `--`. `kept` are the paths
/// the sandbox needs that may lie under `/tmp`, which the private one would otherwise hide:
/// its data and tool scratch writable, its agent executable and tool cache read-only.
pub fn arguments(data: &Path, private_network: bool, kept: &[(&Path, bool)]) -> Vec<OsString> {
    let mut arguments: Vec<OsString> = ["--unshare-user", "--unshare-pid", "--unshare-ipc"].map(Into::into).into();
    if private_network {
        arguments.push("--unshare-net".into());
    }
    // The whole host view stays as it was, under Landlock; only /proc, /dev and /tmp are new.
    for argument in
        ["--as-pid-1", "--cap-drop", "ALL", "--bind", "/", "/", "--proc", "/proc", "--dev", "/dev", "--tmpfs", "/tmp"]
    {
        arguments.push(argument.into());
    }
    arguments.extend(["--bind".into(), data.into(), data.into()]);
    // A tool path appears only once a tool is first asked for; until then there is none to keep.
    for (path, writable) in kept {
        arguments.push(if *writable { "--bind-try" } else { "--ro-bind-try" }.into());
        arguments.push(path.into());
        arguments.push(path.into());
    }
    arguments.extend(["--chdir".into(), data.into(), "--".into()]);
    arguments
}
