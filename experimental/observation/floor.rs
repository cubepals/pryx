// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2
//! Install the real ordinary process and network-off policy in the fixed benchmark child.

use pryx_agent::process_policy::{self, Policy};

#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn pryx_observe_floor(supervisor: u32) -> i32 {
    let data = match std::env::current_dir() {
        Ok(dir) => dir.join("allowed"),
        Err(_) => return 1,
    };
    let policy = Policy {
        data,
        memory_bytes: 512 << 20,
        address_space_bytes: process_policy::DEFAULT_ADDRESS_SPACE_MB * 1024 * 1024,
        open_files: 256,
        processes: 256,
        cpu_seconds: 60,
        nice: 0,
        network_off: true,
        tools: None,
        require_ipc_scope: None,
        private_tmp: None,
    };
    match process_policy::enter(&policy).and_then(|()| process_policy::child(supervisor, true)) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("Guest process floor unavailable: {error}");
            1
        }
    }
}
