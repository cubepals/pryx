// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2
//! Build-only native ABI values from the pinned monitor and its selected libc.
//! This example never creates KVM state or changes any host policy.
fn main() {
    println!(
        "{}",
        serde_json::json!({
            "abort": libc::SIGABRT,
            "epollWait": libc::SYS_epoll_wait,
            "epollPwait": libc::SYS_epoll_pwait,
            "vcpu": vmm::utils::signal::sigrtmin() + vmm::vstate::vcpu::VCPU_RTSIG_OFFSET,
            "futexWait": libc::FUTEX_WAIT,
            "futexJoin": libc::FUTEX_WAIT_BITSET | libc::FUTEX_CLOCK_REALTIME,
            "futexBitsetAny": u32::from_ne_bytes(libc::FUTEX_BITSET_MATCH_ANY.to_ne_bytes()),
        })
    );
}
