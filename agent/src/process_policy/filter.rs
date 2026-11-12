// SPDX-License-Identifier: FSL-1.1-ALv2
//! A deny filter layered on the caller's existing seccomp filter. Clone is allowed only without
//! namespace flags; clone3 returns ENOSYS so libc can use its checked clone fallback.
use seccompiler::{
    BpfProgram, SeccompAction, SeccompCmpArgLen, SeccompCmpOp, SeccompCondition, SeccompFilter, SeccompRule,
};
use std::collections::BTreeMap;

pub(super) fn apply(network_off: bool) -> Result<(), Box<dyn std::error::Error>> {
    variant(network_off, false)
}
pub(super) fn browser() -> Result<(), Box<dyn std::error::Error>> {
    variant(false, true)
}
fn variant(network_off: bool, browser: bool) -> Result<(), Box<dyn std::error::Error>> {
    let mut rules = BTreeMap::new();
    for syscall in [
        libc::SYS_mount,
        libc::SYS_umount2,
        libc::SYS_pivot_root,
        libc::SYS_chroot,
        libc::SYS_ptrace,
        libc::SYS_process_vm_readv,
        libc::SYS_process_vm_writev,
        libc::SYS_pidfd_getfd,
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
    if browser {
        // Namespace operations are confined by the maintained helper's private root and IDs.
        // Ordinary commands retain every original denial and cannot invoke this trusted worker.
        for syscall in [
            libc::SYS_mount,
            libc::SYS_umount2,
            libc::SYS_pivot_root,
            libc::SYS_chroot,
            libc::SYS_unshare,
            libc::SYS_setns,
            libc::SYS_setuid,
            libc::SYS_setgid,
            libc::SYS_setreuid,
            libc::SYS_setregid,
            libc::SYS_setresuid,
            libc::SYS_setresgid,
            libc::SYS_setgroups,
            libc::SYS_capset,
        ] {
            rules.remove(&syscall);
        }
    } else {
        rules.insert(libc::SYS_clone, clone_rules());
    }
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

fn clone_rules() -> Vec<SeccompRule> {
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
    .collect()
}

// Guest-only rules protect every supervisor thread: kill accepts non-leader thread IDs,
// whereas tgkill and queued thread signals require the protected thread-group ID.
pub(super) fn child(supervisor: u32) -> Result<(), Box<dyn std::error::Error>> {
    let mut rules = BTreeMap::new();
    for syscall in [
        libc::SYS_tkill,
        libc::SYS_pidfd_send_signal,
        libc::SYS_setpriority,
        libc::SYS_sched_setscheduler,
        libc::SYS_sched_setparam,
        libc::SYS_sched_setattr,
        libc::SYS_sched_setaffinity,
    ] {
        rules.insert(syscall, vec![]);
    }
    for syscall in [libc::SYS_tgkill, libc::SYS_rt_tgsigqueueinfo] {
        rules.insert(syscall, vec![argument_rule(0, SeccompCmpOp::Eq, supervisor)]);
    }
    let positive_other = SeccompRule::new(vec![
        condition(0, SeccompCmpOp::Le, i32::MAX as u32),
        condition(0, SeccompCmpOp::Ne, 0),
        condition(0, SeccompCmpOp::Ne, std::process::id()),
    ])?;
    let signals = vec![
        positive_other,
        argument_rule(0, SeccompCmpOp::Eq, supervisor),
        argument_rule(0, SeccompCmpOp::Eq, u32::MAX),
        argument_rule(0, SeccompCmpOp::Eq, (-(supervisor as i32)) as u32),
    ];
    rules.insert(libc::SYS_kill, signals.clone());
    rules.insert(libc::SYS_rt_sigqueueinfo, signals);
    rules.insert(libc::SYS_setpgid, vec![argument_rule(1, SeccompCmpOp::Eq, supervisor)]);
    rules.insert(libc::SYS_prlimit64, vec![argument_rule(0, SeccompCmpOp::Ne, 0)]);
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

fn condition(argument: u8, op: SeccompCmpOp, value: u32) -> SeccompCondition {
    SeccompCondition::new(argument, SeccompCmpArgLen::Dword, op, value.into()).expect("valid argument")
}
fn argument_rule(argument: u8, op: SeccompCmpOp, value: u32) -> SeccompRule {
    SeccompRule::new(vec![condition(argument, op, value)]).expect("valid argument rule")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guest_signals_cannot_reach_supervisor_threads() {
        let supervisor = std::process::id();
        let task = std::fs::read_link("/proc/thread-self").unwrap();
        let thread: u32 = task.file_name().unwrap().to_str().unwrap().parse().unwrap();
        let probes = vec![
            vec![libc::SYS_kill, supervisor.into(), 0],
            vec![libc::SYS_kill, thread.into(), 0],
            vec![libc::SYS_kill, -(supervisor as i64), 0],
            vec![libc::SYS_kill, -1, 0],
            vec![libc::SYS_tkill, thread.into(), 0],
            vec![libc::SYS_tgkill, supervisor.into(), thread.into(), 0],
            vec![libc::SYS_rt_sigqueueinfo, supervisor.into(), 0, 0],
            vec![libc::SYS_rt_sigqueueinfo, thread.into(), 0, 0],
            vec![libc::SYS_rt_tgsigqueueinfo, supervisor.into(), thread.into(), 0, 0],
            vec![libc::SYS_pidfd_send_signal, -1, 0, 0, 0],
            vec![libc::SYS_pidfd_getfd, -1, -1, 0],
            vec![libc::SYS_setpgid, -1, supervisor.into()],
            vec![libc::SYS_prlimit64, supervisor.into(), libc::RLIMIT_NOFILE.into(), 0, 0],
        ];
        let probes = probes
            .into_iter()
            .chain(
                [
                    libc::SYS_setpriority,
                    libc::SYS_sched_setscheduler,
                    libc::SYS_sched_setparam,
                    libc::SYS_sched_setattr,
                    libc::SYS_sched_setaffinity,
                ]
                .into_iter()
                .map(|syscall| vec![syscall, -1, -1, 0]),
            )
            .collect::<Vec<_>>();
        let script = r#"
import ctypes, errno, json, os, resource
libc = ctypes.CDLL(None, use_errno=True)
for probe in json.loads(os.environ['PROBES']):
    ctypes.set_errno(0)
    result = libc.syscall(*probe)
    assert result == -1 and ctypes.get_errno() == errno.EPERM, (probe, result, ctypes.get_errno())
# libc's ordinary self-limit query remains available.
resource.getrlimit(resource.RLIMIT_NOFILE)
"#;
        let output = std::thread::spawn(move || {
            apply(false).unwrap();
            child(supervisor).unwrap();
            std::process::Command::new("python3")
                .args(["-c", script])
                .env("PROBES", serde_json::to_string(&probes).unwrap())
                .output()
                .unwrap()
        })
        .join()
        .unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    }

    #[test]
    fn guest_tgkill_can_still_signal_its_child() {
        let mut disposable = std::process::Command::new("/bin/sleep").arg("30").spawn().unwrap();
        let target = disposable.id();
        let result = std::thread::spawn(move || {
            apply(false).unwrap();
            child(std::process::id()).unwrap();
            std::process::Command::new("python3").args(["-c",
                "import ctypes,os; assert ctypes.CDLL(None).syscall(int(os.environ['SIGNAL_CALL']),int(os.environ['TARGET']),int(os.environ['TARGET']),0)==0"])
                .env("SIGNAL_CALL", libc::SYS_tgkill.to_string()).env("TARGET", target.to_string()).status().unwrap()
        }).join();
        disposable.kill().unwrap();
        disposable.wait().unwrap();
        assert!(result.unwrap().success());
    }
}
