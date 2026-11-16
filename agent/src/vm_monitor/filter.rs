// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2
//! The monitor inherits its kernel lease on stdin through the unchanged jailer. Prevent
//! closing, overwriting, unlocking or marking that descriptor close-on-exec. Every monitor
//! is denied cross-process signals and limit changes; ordinary monitors also cannot create
//! network sockets.
//! Packet proxies inherit one private listener and may create only outbound Internet sockets and
//! routing netlink sockets, which ask the kernel how a destination is routed; without
//! CAP_NET_ADMIN, which a packet proxy never holds, those read the host's routes and change none.
//! Filesystem programs create no socket at all.
use seccompiler::{
    BpfProgram, SeccompAction, SeccompCmpArgLen, SeccompCmpOp, SeccompCondition, SeccompFilter, SeccompRule,
};
use std::collections::BTreeMap;

fn condition(argument: u8, operation: SeccompCmpOp, value: u32) -> SeccompCondition {
    SeccompCondition::new(argument, SeccompCmpArgLen::Dword, operation, value.into()).expect("valid monitor condition")
}
fn rule(argument: u8, operation: SeccompCmpOp, value: u32) -> SeccompRule {
    SeccompRule::new(vec![condition(argument, operation, value)]).expect("valid monitor rule")
}

/// The socket domains a confined process may create.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Sockets {
    Unix,
    Internet,
    None,
}

pub(super) fn enter(jailer: bool, sockets: Sockets) -> Result<(), Box<dyn std::error::Error>> {
    let mut rules = BTreeMap::new();
    let input = libc::STDIN_FILENO as u32;
    rules.insert(libc::SYS_close, vec![rule(0, SeccompCmpOp::Eq, input)]);
    rules.insert(libc::SYS_ioctl, vec![rule(0, SeccompCmpOp::Eq, input)]);
    // The host selects synchronous block I/O. An asynchronous close must not
    // bypass the lease descriptor's syscall restrictions, including under the jailer.
    rules.insert(libc::SYS_io_uring_setup, vec![]);
    rules.insert(libc::SYS_close_range, vec![rule(0, SeccompCmpOp::Eq, input)]);
    let overwrite_input =
        SeccompRule::new(vec![condition(1, SeccompCmpOp::Eq, input), condition(0, SeccompCmpOp::Ne, input)])?;
    rules.insert(libc::SYS_dup3, vec![overwrite_input.clone()]);
    #[cfg(target_arch = "x86_64")]
    rules.insert(libc::SYS_dup2, vec![overwrite_input]);
    rules.insert(libc::SYS_flock, vec![]);
    rules.insert(
        libc::SYS_fcntl,
        vec![SeccompRule::new(vec![
            condition(0, SeccompCmpOp::Eq, input),
            condition(1, SeccompCmpOp::Eq, libc::F_SETFD as u32),
            condition(2, SeccompCmpOp::MaskedEq(libc::FD_CLOEXEC as u64), libc::FD_CLOEXEC as u32),
        ])?],
    );
    for syscall in [
        libc::SYS_kill,
        libc::SYS_tkill,
        libc::SYS_rt_sigqueueinfo,
        libc::SYS_pidfd_send_signal,
        libc::SYS_setpriority,
        libc::SYS_sched_setparam,
        libc::SYS_sched_setattr,
        libc::SYS_sched_setaffinity,
        libc::SYS_sched_setscheduler,
    ] {
        rules.insert(syscall, vec![]);
    }
    for syscall in [libc::SYS_tgkill, libc::SYS_rt_tgsigqueueinfo] {
        rules.insert(syscall, vec![rule(0, SeccompCmpOp::Ne, std::process::id())]);
    }
    rules.insert(
        libc::SYS_prlimit64,
        vec![SeccompRule::new(vec![
            condition(0, SeccompCmpOp::Ne, 0),
            condition(0, SeccompCmpOp::Ne, std::process::id()),
        ])?],
    );
    if sockets == Sockets::None {
        for syscall in [libc::SYS_socket, libc::SYS_socketpair] {
            rules.insert(syscall, vec![]);
        }
    } else if !jailer {
        let packet_proxy = sockets == Sockets::Internet;
        let domains: &[i32] =
            if packet_proxy { &[libc::AF_INET, libc::AF_INET6, libc::AF_NETLINK] } else { &[libc::AF_UNIX] };
        let mut refused = vec![SeccompRule::new(
            domains.iter().map(|domain| condition(0, SeccompCmpOp::Ne, *domain as u32)).collect(),
        )?];
        if packet_proxy {
            // Of netlink, only the routing family: the helper's admission asks it how a flow's
            // destination would be routed, and no other family answers that.
            refused.push(SeccompRule::new(vec![
                condition(0, SeccompCmpOp::Eq, libc::AF_NETLINK as u32),
                condition(2, SeccompCmpOp::Ne, libc::NETLINK_ROUTE as u32),
            ])?);
        }
        rules.insert(libc::SYS_socket, refused);
        if packet_proxy {
            for syscall in [libc::SYS_socketpair, libc::SYS_bind, libc::SYS_listen] {
                rules.insert(syscall, vec![]);
            }
        }
    }
    let program: BpfProgram = SeccompFilter::new(
        rules,
        SeccompAction::Allow,
        SeccompAction::Errno(libc::EPERM as u32),
        std::env::consts::ARCH.try_into()?,
    )?
    .try_into()?;
    crate::syscall_abi::apply(&program)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn jailed_monitor_cannot_signal_its_parent() {
        const CHILD: &str = "PRYX_JAILED_SIGNAL_FILTER_CHILD";
        if std::env::var_os(CHILD).is_some() {
            let parent = rustix::process::getppid().unwrap();
            assert!(rustix::process::test_kill_process(parent).is_ok());
            super::enter(true, super::Sockets::Unix).unwrap();
            assert_eq!(rustix::process::test_kill_process(parent), Err(rustix::io::Errno::PERM));
            return;
        }
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "vm_monitor::filter::tests::jailed_monitor_cannot_signal_its_parent"])
            .env(CHILD, "on")
            .output()
            .unwrap();
        assert!(
            child.status.success(),
            "{}{}",
            String::from_utf8_lossy(&child.stdout),
            String::from_utf8_lossy(&child.stderr)
        );
    }
}
