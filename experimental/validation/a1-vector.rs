// SPDX-License-Identifier: FSL-1.1-ALv2
// This test-only C ABI is appended to an isolated build of the pinned A1 crate.
// The sampler selects the same runtime ISA prerequisites as the A1 Rust vector backend.

#[unsafe(no_mangle)]
pub extern "C" fn a1_vector_available() -> bool {
    Backend::available().iter().any(|backend| backend.name() == "rust-vector")
}

/// # Safety
/// The caller has checked a1_vector_available. Both ranges are valid for length bytes
/// and are disjoint, as required by the original copy method.
#[unsafe(no_mangle)]
#[inline(never)]
pub unsafe extern "C" fn a1_vector_copy(dst: *mut u8, src: *const u8, length: usize, _: usize) {
    let dst = unsafe { std::slice::from_raw_parts_mut(dst, length) };
    let src = unsafe { std::slice::from_raw_parts(src, length) };
    unsafe { vector::transfer(dst, Some(src)) };
}

/// # Safety
/// The caller has checked a1_vector_available. The destination is valid for length bytes.
#[unsafe(no_mangle)]
#[inline(never)]
pub unsafe extern "C" fn a1_vector_clear(dst: *mut u8, _: *const u8, length: usize, _: usize) {
    let dst = unsafe { std::slice::from_raw_parts_mut(dst, length) };
    unsafe { vector::transfer(dst, None) };
}
