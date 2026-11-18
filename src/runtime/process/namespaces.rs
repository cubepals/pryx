// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! The namespaced process mode. Where this host lets the sandbox's user create a user namespace
//! and use capabilities in it, each supervisor starts through the installation's unmodified
//! bubblewrap, as PID 1 of its own PID namespace, with its own mount and IPC namespaces, a private
//! `/tmp` and, for `network: none`, an empty network namespace of its own. Landlock, seccomp and
//! the limits stay what the supervisor installs inside it. Where the helper is missing or the
//! probe fails, the tier is exactly the plain mode and says why.
//!
//! The probe is the launch itself: the same helper, the same arguments, as the sandbox's own
//! user, running the supervisor's executable. One answer per executable for the runtime's life,
//! so what the node announces and what a launch does never differ; a launch whose helper fails
//! then fails, and never starts plain instead. Not for listener passing or signals, which stay in
//! power.rs, nor for the browser's own nested helper, which the agent starts.
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

/// bubblewrap beside the agent executable; its one library is `../lib` from it (its RPATH).
pub const HELPER: &str = "namespaces/bin/bwrap";
/// How long the probe's own sandbox may take to start and print the supervisor's version.
const PROBE: Duration = Duration::from_secs(10);

/// Why a sandbox that started in its own namespaces isn't relaunched: it may have been placed for
/// them, so it keeps its first launch's mode or doesn't start.
pub(super) const RELAUNCH_WITHOUT_NAMESPACES: &str = "this sandbox started in namespaces of its own and is not relaunched without them; the namespace helper no longer starts here";

/// The mode a sandbox was started in, as its runtime journal records it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Namespaced {
    pub(crate) private_network: bool,
}

/// The installation's helper for `agent`, if it is a protected regular file.
pub(crate) fn helper(agent: &Path) -> Result<PathBuf, String> {
    use std::os::unix::fs::MetadataExt;
    let path = agent.parent().ok_or("The agent has no installation directory")?.join(HELPER);
    let info = std::fs::symlink_metadata(&path)
        .map_err(|_| "This installation has no namespace helper; install a release that carries it".to_string())?;
    let own = rustix::process::geteuid().as_raw();
    if !info.is_file() || info.mode() & 0o022 != 0 || (info.uid() != own && info.uid() != 0) {
        return Err("The namespace helper is not a protected installation file".into());
    }
    Ok(path)
}

/// bubblewrap's arguments for one supervisor: the agent crate's, which its own tests start too.
pub(super) use pryx_agent::process_namespaces::arguments;

/// A namespaced sandbox's private /tmp size: the node's /tmp size for a sandbox, no more than its
/// memory, which the RSS watchdog would never see spent there, and never 0, which is no limit.
pub(super) fn tmp_bytes(tmp_size_mb: u32, memory_bytes: i64) -> u64 {
    const MIB: u64 = 1 << 20;
    let memory = u64::try_from(memory_bytes).ok().filter(|bytes| *bytes > 0).unwrap_or(u64::MAX);
    (u64::from(tmp_size_mb.max(1)) * MIB).min(memory)
}

/// The helper for `agent`, once a launch-equivalent probe as `owner` has proved it here.
pub(crate) fn proved(agent: &Path, owner: (u32, u32)) -> Result<PathBuf, String> {
    let helper = helper(agent)?;
    probe(&helper, agent, owner)?;
    Ok(helper)
}

/// Runs the helper exactly as a launch would, as `owner`, with the supervisor printing its
/// version. Every namespace a launch may take is in it, the network one included.
fn probe(helper: &Path, agent: &Path, owner: (u32, u32)) -> Result<(), String> {
    use std::os::unix::process::CommandExt;
    let mut command = Command::new(helper);
    command.args(arguments(Path::new("/"), true, tmp_bytes(1, 0), &[(agent, false)])).arg(agent).arg("--version");
    command.env_clear().stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    if rustix::process::geteuid().is_root() {
        command.uid(owner.0).gid(owner.1);
    }
    // As a launch does: a daemon granted ambient capabilities for kernel network policy passes
    // none on, and bubblewrap refuses to start with capabilities it wasn't installed with.
    crate::bpf::host::drop_ambient_on_exec(&mut command);
    let mut child = command.spawn().map_err(|error| format!("The namespace helper did not start: {error}"))?;
    let deadline = std::time::Instant::now() + PROBE;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if std::time::Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("The namespace helper did not finish its probe".into());
            }
        }
    }
    let output = child.wait_with_output().map_err(|error| error.to_string())?;
    let said = String::from_utf8_lossy(&output.stdout);
    if output.status.success() && said.starts_with("pryx-agent ") {
        return Ok(());
    }
    let reason = String::from_utf8_lossy(&output.stderr);
    Err(format!("user namespaces are unavailable to the namespace helper: {}", reason.trim()))
}

/// The host probe's entry for the namespaced mode, from the launch-equivalent probe.
pub(crate) fn capability(agent: &Path, probed: Result<(), String>) -> crate::capability::Capability {
    use crate::capability::{Capability, State};
    match probed {
        Ok(()) => Capability::on("Process sandboxes start in their own user, mount, PID and IPC namespaces"),
        Err(reason) => {
            let action = action(agent, restricted() && userns_denial(&reason));
            Capability::new(State::Off, reason, action)
        }
    }
}

/// bubblewrap 0.13.1's raw_clone failure and rtnetlink diagnostics (bubblewrap.c and
/// network.c). These are its actual C messages, without Rust io::Error's display suffix.
const USERNS_DENIALS: &[&str] = &[
    "No permissions to create a new namespace",
    "Creating new namespace failed: Operation not permitted",
    "loopback: Failed RTM_NEWADDR: Operation not permitted",
];

/// Only a launch-equivalent namespace refusal under Ubuntu's rule suggests an AppArmor profile.
/// A missing helper, spawn failure, timeout or unrelated tool failure needs its own action.
fn userns_denial(reason: &str) -> bool {
    reason.starts_with("user namespaces are unavailable to the namespace helper:")
        && USERNS_DENIALS.iter().any(|diagnostic| reason.contains(diagnostic))
}

/// Ubuntu 24.04 and later let only binaries an AppArmor profile names use a user namespace
/// (`kernel.apparmor_restrict_unprivileged_userns`). The `user` preset never installs one.
fn restricted() -> bool {
    std::fs::read_to_string("/proc/sys/kernel/apparmor_restrict_unprivileged_userns").is_ok_and(|on| on.trim() == "1")
}

/// What would turn the mode on: under Ubuntu's rule, the one profile root installs for this
/// installation's helper, printed whole, with the two commands that load it.
pub fn action(agent: &Path, restricted: bool) -> String {
    let helper = agent.parent().unwrap_or(Path::new("/")).join(HELPER);
    if !restricted {
        return "Allow unprivileged user namespaces on this host (user.max_user_namespaces above 0)".into();
    }
    let warning = if root_protected(&helper) {
        ""
    } else {
        "Warning: this helper path is writable by an ordinary user; its owner can replace the binary that receives the profile. Move the installation to a root-owned directory before granting it. "
    };
    format!(
        "{warning}As root, save this AppArmor profile as /etc/apparmor.d/pryx-namespaces, then run \
         apparmor_parser -r /etc/apparmor.d/pryx-namespaces and restart pryx:\n{}",
        profile(&helper)
    )
}

/// An unconfined profile that lets exactly this helper create user namespaces, as Ubuntu's own
/// profiles do for the programs it ships.
pub fn profile(helper: &Path) -> String {
    format!(
        "abi <abi/4.0>,\ninclude <tunables/global>\n\nprofile pryx-namespaces {} flags=(unconfined) {{\n  userns,\n\n  include if exists <local/pryx-namespaces>\n}}\n",
        quoted_attachment(helper)
    )
}

fn root_protected(path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    path.ancestors().all(|part| {
        std::fs::symlink_metadata(part)
            .is_ok_and(|info| !info.file_type().is_symlink() && info.uid() == 0 && info.mode() & 0o022 == 0)
    })
}

/// AppArmor attachment names are quoted and AARE metacharacters are literal, so an installation
/// path cannot widen this profile to another program or inject another profile rule.
fn quoted_attachment(path: &Path) -> String {
    let mut quoted = String::from("\"");
    for c in path.to_string_lossy().chars() {
        match c {
            '\\' | '\"' | '*' | '?' | '[' | ']' | '{' | '}' | '@' => {
                quoted.push('\\');
                quoted.push(c);
            }
            '\n' => quoted.push_str("\\012"),
            '\r' => quoted.push_str("\\015"),
            '\t' => quoted.push_str("\\011"),
            c => quoted.push(c),
        }
    }
    quoted.push('\"');
    quoted
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn a_launch_takes_every_namespace_and_a_private_tmp_and_only_network_none_its_own_network() {
        let data = Path::new("/state/data/sandbox");
        let agent = Path::new("/tmp/install/pryx-agent");
        let shared = arguments(data, false, tmp_bytes(256, 64 << 20), &[(agent, false)]);
        let text =
            |arguments: &[OsString]| arguments.iter().map(|a| a.to_string_lossy().into_owned()).collect::<Vec<_>>();
        let shared = text(&shared);
        for required in ["--unshare-user", "--unshare-pid", "--unshare-ipc", "--as-pid-1", "--proc", "--dev"] {
            assert!(shared.contains(&required.to_string()), "{required}: {shared:?}");
        }
        let bound = (64u64 << 20).to_string();
        assert!(
            shared.windows(4).any(|w| w == ["--size", bound.as_str(), "--tmpfs", "/tmp"]),
            "bounded by memory: {shared:?}"
        );
        assert!(shared.windows(2).any(|pair| pair == ["--cap-drop", "ALL"]), "{shared:?}");
        assert!(!shared.contains(&"--unshare-net".to_string()), "network internet shares the host's: {shared:?}");
        assert_eq!(shared.last().map(String::as_str), Some("--"));
        assert!(shared.windows(2).any(|pair| pair == ["--chdir", "/state/data/sandbox"]), "{shared:?}");
        let tmpfs = shared.iter().position(|a| a == "/tmp").unwrap();
        let kept = shared.iter().rposition(|a| a == "/tmp/install/pryx-agent").unwrap();
        assert!(kept > tmpfs, "what lies under /tmp is bound after the private one: {shared:?}");
        assert!(shared.windows(3).any(|w| w == ["--bind", "/state/data/sandbox", "/state/data/sandbox"]));
        assert!(
            shared.windows(3).any(|w| w == ["--ro-bind-try", "/tmp/install/pryx-agent", "/tmp/install/pryx-agent"])
        );
        let private = text(&arguments(data, true, tmp_bytes(0, 0), &[]));
        assert!(private.windows(2).any(|w| w == ["--size", (1u64 << 20).to_string().as_str()]), "never 0: {private:?}");
        assert!(private.contains(&"--unshare-net".to_string()), "{private:?}");
        for refused in ["--die-with-parent", "--cap-add", "--disable-userns"] {
            assert!(!private.contains(&refused.to_string()), "{refused}: {private:?}");
        }
    }

    #[test]
    fn under_ubuntus_rule_the_action_is_the_profile_for_this_installations_helper() {
        let agent = Path::new("/home/someone/.local/lib/pryx/pryx-agent");
        let said = action(agent, true);
        assert!(said.contains("apparmor_parser -r /etc/apparmor.d/pryx-namespaces"), "{said}");
        assert!(
            said.contains("profile pryx-namespaces \"/home/someone/.local/lib/pryx/namespaces/bin/bwrap\""),
            "{said}"
        );
        assert!(said.contains("userns,") && said.contains("flags=(unconfined)"), "{said}");
        assert!(said.contains("writable by an ordinary user"), "{said}");
        assert!(!action(agent, false).contains("apparmor"), "no profile where the rule is off");
    }

    #[test]
    fn only_a_namespace_permission_denial_suggests_the_profile() {
        // Recorded real helper output: Ubuntu's loopback denial and raw_clone's own message.
        for denied in [
            "user namespaces are unavailable to the namespace helper: bwrap: loopback: Failed RTM_NEWADDR: Operation not permitted",
            "user namespaces are unavailable to the namespace helper: bwrap: No permissions to create a new namespace, likely because the kernel does not allow non-privileged user namespaces.",
        ] {
            assert!(userns_denial(denied));
        }
        for unrelated in [
            "This installation has no namespace helper",
            "The namespace helper did not finish its probe",
            "The namespace helper did not start: Permission denied",
            "user namespaces are unavailable to the namespace helper: wrong executable",
        ] {
            assert!(!userns_denial(unrelated), "{unrelated}");
        }
    }

    #[test]
    fn the_attachment_quotes_spaces_and_escapes_profile_metacharacters() {
        let path = Path::new("/opt/a b/\"quoted\"*?[{}]/bwrap");
        assert_eq!(quoted_attachment(path), "\"/opt/a b/\\\"quoted\\\"\\*\\?\\[\\{\\}\\]/bwrap\"");
        let text = profile(path);
        assert!(text.contains(&quoted_attachment(path)), "{text}");
    }

    #[test]
    fn the_helper_must_be_a_protected_installation_file() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let agent = root.path().join("pryx-agent");
        assert!(helper(&agent).unwrap_err().contains("no namespace helper"));
        let path = root.path().join(HELPER);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"#!/bin/false\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o775)).unwrap();
        assert!(helper(&agent).unwrap_err().contains("protected"));
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(helper(&agent).unwrap(), path);
        let target = root.path().join("elsewhere");
        std::fs::rename(&path, &target).unwrap();
        std::os::unix::fs::symlink(&target, &path).unwrap();
        assert!(helper(&agent).is_err(), "a link is not the installation's file");
    }
}
