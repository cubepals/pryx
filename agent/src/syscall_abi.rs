// SPDX-License-Identifier: FSL-1.1-ALv2
//! Native syscall ABI checks around the maintained seccompiler's generated policies.
//! Its architecture check rejects other audit architectures, but x32 shares x86_64's audit
//! architecture and adds a syscall-number bit. Deny that ABI before native-number rules run.
use seccompiler::{BpfProgram, Error, sock_filter};

#[cfg(target_arch = "x86_64")]
fn native(program: &[sock_filter]) -> BpfProgram {
    // seccomp(2): the x32 ABI sets this kernel UAPI bit while keeping AUDIT_ARCH_X86_64.
    // seccompiler's rule API matches syscall numbers, not number-bit predicates. This
    // small policy prefix uses its exported instruction type and unchanged installer.
    let mut guarded = vec![
        sock_filter {
            code: (libc::BPF_LD | libc::BPF_W | libc::BPF_ABS) as u16,
            jt: 0,
            jf: 0,
            k: std::mem::offset_of!(libc::seccomp_data, nr) as u32,
        },
        sock_filter {
            code: (libc::BPF_JMP | libc::BPF_JSET | libc::BPF_K) as u16,
            jt: 0,
            jf: 1,
            k: linux_raw_sys::general::__X32_SYSCALL_BIT,
        },
        sock_filter {
            code: (libc::BPF_RET | libc::BPF_K) as u16,
            jt: 0,
            jf: 0,
            k: seccompiler::SeccompAction::Errno(libc::EPERM as u32).into(),
        },
    ];
    guarded.extend_from_slice(program);
    guarded
}

#[cfg(not(target_arch = "x86_64"))]
fn native(program: &[sock_filter]) -> BpfProgram {
    program.to_vec()
}

pub(crate) fn apply(program: &[sock_filter]) -> Result<(), Error> {
    seccompiler::apply_filter(&native(program))
}

pub(crate) fn apply_all_threads(program: &[sock_filter]) -> Result<(), Error> {
    seccompiler::apply_filter_all_threads(&native(program))
}
