// SPDX-License-Identifier: FSL-1.1-ALv2
//! C calls exactly one Rust entry. The trusted VM supervisor retains its PID-1,
//! identity, network, mount, sleep and shutdown checks in the existing module.

#[expect(unsafe_code, reason = "the single C entry has a unique lab symbol and the C signature below")]
#[unsafe(no_mangle)]
pub extern "C" fn pryx_lab_supervise() -> std::ffi::c_int {
    match std::panic::catch_unwind(pryx_agent::vm_init::run) {
        Ok(Ok(())) => 0,
        Ok(Err(_)) | Err(_) => 1,
    }
}

// The kernel-owned argument vector is bounded by the same one-shot bootstrap.
// No workload runs before this credential drop or sees the inherited descriptors.
#[cfg(feature = "lab-guest-supervisor-gate")]
fn start_guest_service() -> Result<(), Box<dyn std::error::Error>> {
    use std::io::Read;
    let mut encoded = Vec::new();
    std::fs::File::open("/proc/self/cmdline")?
        .take(pryx_agent::vm_boot::MAX_BYTES as u64 + 1)
        .read_to_end(&mut encoded)?;
    if encoded.len() > pryx_agent::vm_boot::MAX_BYTES || encoded.last() != Some(&0) {
        return Err("Invalid VM child arguments".into());
    }
    let fields = encoded[..encoded.len() - 1]
        .split(|b| *b == 0)
        .map(|part| Ok(std::str::from_utf8(part)?.to_owned()))
        .collect::<Result<Vec<_>, std::str::Utf8Error>>()?;
    if fields.get(1).is_none_or(|mode| mode != pryx_agent::vm_boot::AGENT_MODE) {
        return Err("Invalid VM child role".into());
    }
    pryx_agent::vm_init::agent(&fields[2..])
}

#[cfg(feature = "lab-guest-supervisor-gate")]
#[expect(unsafe_code, reason = "this is the single fixed entry for the C VM child launcher")]
#[unsafe(no_mangle)]
pub extern "C" fn pryx_lab_vm_agent() -> std::ffi::c_int {
    match std::panic::catch_unwind(start_guest_service) {
        Ok(Ok(())) => 0,
        Ok(Err(_)) | Err(_) => 1,
    }
}
