// SPDX-License-Identifier: FSL-1.1-ALv2
//! A deny filter layered on the caller's existing seccomp filter. Clone is allowed only without
//! namespace flags; clone3 returns ENOSYS so libc can use its checked clone fallback.
use seccompiler::{
    BpfProgram, SeccompAction, SeccompCmpArgLen, SeccompCmpOp, SeccompCondition, SeccompFilter, SeccompRule,
};
use std::collections::BTreeMap;

pub(super) fn apply(network_off: bool) -> Result<(), Box<dyn std::error::Error>> {
    let mut rules = BTreeMap::new();
    for syscall in [
        libc::SYS_mount,
        libc::SYS_umount2,
        libc::SYS_pivot_root,
        libc::SYS_chroot,
        libc::SYS_ptrace,
        libc::SYS_process_vm_readv,
        libc::SYS_process_vm_writev,
        libc::SYS_keyctl,
        libc::SYS_add_key,
        libc::SYS_request_key,
        libc::SYS_bpf,
        libc::SYS_init_module,
        libc::SYS_finit_module,
        libc::SYS_delete_module,
        libc::SYS_unshare,
        libc::SYS_setns,
        libc::SYS_reboot,
        libc::SYS_kexec_load,
        libc::SYS_open_by_handle_at,
        libc::SYS_name_to_handle_at,
        libc::SYS_perf_event_open,
        libc::SYS_userfaultfd,
        libc::SYS_io_uring_setup,
        libc::SYS_swapon,
        libc::SYS_swapoff,
        libc::SYS_setuid,
        libc::SYS_setgid,
        libc::SYS_setreuid,
        libc::SYS_setregid,
        libc::SYS_setresuid,
        libc::SYS_setresgid,
        libc::SYS_setgroups,
        libc::SYS_capset,
    ] {
        rules.insert(syscall, vec![]);
    }
    rules.insert(
        libc::SYS_clone,
        [
            libc::CLONE_NEWUSER,
            libc::CLONE_NEWNS,
            libc::CLONE_NEWPID,
            libc::CLONE_NEWNET,
            libc::CLONE_NEWIPC,
            libc::CLONE_NEWUTS,
            libc::CLONE_NEWCGROUP,
        ]
        .into_iter()
        .map(|flag| {
            SeccompRule::new(vec![
                SeccompCondition::new(0, SeccompCmpArgLen::Qword, SeccompCmpOp::MaskedEq(flag as u64), flag as u64)
                    .expect("valid clone mask"),
            ])
            .expect("valid clone rule")
        })
        .collect(),
    );
    if network_off {
        for syscall in [libc::SYS_connect, libc::SYS_bind, libc::SYS_sendto, libc::SYS_sendmsg, libc::SYS_sendmmsg] {
            rules.insert(syscall, vec![]);
        }
    }
    let architecture = std::env::consts::ARCH.try_into()?;
    let program: BpfProgram =
        SeccompFilter::new(rules, SeccompAction::Allow, SeccompAction::Errno(libc::EPERM as u32), architecture)?
            .try_into()?;
    seccompiler::apply_filter(&program)?;
    let clone3: BpfProgram = SeccompFilter::new(
        [(libc::SYS_clone3, vec![])].into_iter().collect(),
        SeccompAction::Allow,
        SeccompAction::Errno(libc::ENOSYS as u32),
        architecture,
    )?
    .try_into()?;
    seccompiler::apply_filter(&clone3)?;
    Ok(())
}
