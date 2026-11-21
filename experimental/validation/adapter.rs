// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2
//! Export the release-built production scan ABI for bounded symbolic execution.

/// # Safety
/// `ptr` points to a live slice of at least `length` bytes.
#[unsafe(no_mangle)]
#[inline(never)]
pub unsafe extern "C" fn pryx_prove_scan(ptr: *const u8, length: usize) -> u32 {
    let bytes = unsafe { std::slice::from_raw_parts(ptr, length) };
    pryx_lab_proof::all_zero(bytes).into()
}

fn main() {
    // Keep the unmodified exported function reachable when the linker collects sections.
    std::hint::black_box(pryx_prove_scan as unsafe extern "C" fn(*const u8, usize) -> u32);
}
