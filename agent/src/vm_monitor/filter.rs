// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2
//! The monitor inherits its kernel lease on stdin through the unchanged jailer. Prevent
//! closing, overwriting, unlocking or marking that descriptor close-on-exec. Ordinary-user
//! monitors also cannot signal another process, change its limits or create network sockets.
//! Packet proxies inherit one private listener and may create only outbound Internet sockets.
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

pub(super) fn enter(jailer: bool, packet_proxy: bool) -> Result<(), Box<dyn std::error::Error>> {
    let mut rules = BTreeMap::new();
    let input = libc::STDIN_FILENO as u32;
    rules.insert(libc::SYS_close, vec![rule(0, SeccompCmpOp::Eq, input)]);
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
    if !jailer {
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
        let domains: &[i32] = if packet_proxy { &[libc::AF_INET, libc::AF_INET6] } else { &[libc::AF_UNIX] };
        rules.insert(
            libc::SYS_socket,
            vec![SeccompRule::new(
                domains.iter().map(|domain| condition(0, SeccompCmpOp::Ne, *domain as u32)).collect(),
            )?],
        );
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
    seccompiler::apply_filter(&program)?;
    Ok(())
}
