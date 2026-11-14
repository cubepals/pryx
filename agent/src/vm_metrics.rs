// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2
//! Read guest kernel counters through the existing procfs library. Only trusted VM bootstrap
//! enables sampling; process, Wasm and maintenance agents expose no host-wide counters here.
//! These observations neither enforce limits nor grant a request any host capability.
use crate::state::VmResources;
use procfs::{Current, CurrentSI};
use std::time::Duration;

fn cpu() -> Option<u64> {
    let cpu = procfs::KernelStats::current().ok()?.total;
    // Guest and guest_nice are already included in user/nice. Idle, I/O wait and stolen
    // host scheduling time are not CPU consumed by this guest.
    let time = [
        cpu.user_duration(),
        cpu.nice_duration(),
        cpu.system_duration(),
        cpu.irq_duration().unwrap_or_default(),
        cpu.softirq_duration().unwrap_or_default(),
    ]
    .into_iter()
    .try_fold(Duration::ZERO, |sum, time| sum.checked_add(time))?;
    u64::try_from(time.as_nanos()).ok()
}
pub(crate) fn snapshot(enabled: bool) -> Option<VmResources> {
    if !enabled {
        return None;
    }
    let memory = procfs::Meminfo::current().ok();
    let network = procfs::net::dev_status().ok().and_then(|interfaces| {
        interfaces.values().try_fold((0_u64, 0_u64), |(rx, tx), interface| {
            Some((rx.checked_add(interface.recv_bytes)?, tx.checked_add(interface.sent_bytes)?))
        })
    });
    // Linux admin-guide/cgroup-v2.rst defines pids.current as a decimal subtree count.
    let pids = std::fs::read_to_string(std::path::Path::new(crate::vm_boot::COMMAND_CGROUP).join("pids.current"))
        .ok()
        .and_then(|value| value.trim().parse().ok());
    Some(VmResources {
        cpu_total_ns: cpu(),
        memory_usage: memory.as_ref().and_then(|memory| memory.mem_total.checked_sub(memory.mem_free)),
        memory_inactive_file: memory.as_ref().and_then(|memory| memory.inactive_file),
        pids,
        rx_bytes: network.map(|(rx, _)| rx),
        tx_bytes: network.map(|(_, tx)| tx),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn real_kernel_counters_use_library_units_and_other_tiers_are_absent() {
        assert!(snapshot(false).is_none());
        let observed = snapshot(true).unwrap();
        let memory = procfs::Meminfo::current().unwrap();
        assert!(observed.memory_usage.unwrap() > 0);
        assert!(observed.memory_usage.unwrap() <= memory.mem_total);
        assert!(observed.memory_inactive_file.unwrap() <= memory.mem_total);
        assert!(observed.cpu_total_ns.unwrap() > 0);
        let later = snapshot(true).unwrap();
        assert!(later.cpu_total_ns.unwrap() >= observed.cpu_total_ns.unwrap());
        assert!(later.rx_bytes.unwrap() >= observed.rx_bytes.unwrap());
        assert!(later.tx_bytes.unwrap() >= observed.tx_bytes.unwrap());
    }
}
