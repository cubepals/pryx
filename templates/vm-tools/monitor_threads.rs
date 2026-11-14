// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2
//! Execute the pinned monitor's real thread signalling/join ABI under its compiled filters.
//! Signals are blocked before creating the threads; this starts no guest and needs no KVM.
use std::{
    fs::File,
    sync::{Arc, Barrier},
    thread,
    time::{Duration, Instant},
};
use vmm_sys_util::signal::{Killable, block_signal};

fn main() {
    let path = std::env::args().nth(1).expect("compiled native policy");
    let filters = vmm::seccomp::deserialize_binary(File::open(path).unwrap()).unwrap();
    let signal = vmm::utils::signal::sigrtmin() + vmm::vstate::vcpu::VCPU_RTSIG_OFFSET;
    block_signal(signal).unwrap();
    let roles = ["api", "vcpu"];
    assert_eq!(filters.len(), roles.len() + 1);
    let barrier = Arc::new(Barrier::new(roles.len() + 1));
    let mut threads = Vec::new();
    for role in roles {
        let policy = filters[role].clone();
        let phase = barrier.clone();
        threads.push(thread::spawn(move || {
            vmm::seccomp::apply_filter(&policy).unwrap();
            phase.wait();
            phase.wait();
            let until = Instant::now() + Duration::from_millis(10);
            while Instant::now() < until {
                std::hint::spin_loop();
            }
        }));
    }
    vmm::seccomp::apply_filter(&filters["vmm"]).unwrap();
    barrier.wait();
    for child in &threads {
        child.kill(signal).unwrap();
    }
    barrier.wait();
    for child in threads {
        child.join().unwrap();
    }
}
