// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

//! CPU-specific assembly declarations. The caller checks ISA and slice bounds.
//! Parts (`src/`): `lib.rs` binds the ABI, `x86_64.S` and `aarch64.S` implement it.
//! Miri uses the scalar implementation in the caller and links no assembly.

#![cfg(all(not(miri), target_os = "linux", target_endian = "little"))]

#[cfg(target_arch = "x86_64")]
core::arch::global_asm!(include_str!("x86_64.S"));
#[cfg(target_arch = "aarch64")]
core::arch::global_asm!(include_str!("aarch64.S"));

#[cfg(target_arch = "x86_64")]
unsafe extern "C" {
    pub fn pryx_page_diff_avx2(a: *const u8, b: *const u8, len: usize) -> usize;
    pub fn pryx_page_value_avx2(a: *const u8, value: u8, len: usize) -> usize;
    pub fn pryx_page_diff_avx512(a: *const u8, b: *const u8, len: usize) -> usize;
    pub fn pryx_page_value_avx512(a: *const u8, value: u8, len: usize) -> usize;
    pub fn pryx_page_copy_gpr(dst: *mut u8, src: *const u8, len: usize, prefetch: usize);
    pub fn pryx_page_clear_gpr(dst: *mut u8, unused: *const u8, len: usize, unused_hint: usize);
    pub fn pryx_page_copy_rep(dst: *mut u8, src: *const u8, len: usize, unused_hint: usize);
    pub fn pryx_page_clear_rep(dst: *mut u8, unused: *const u8, len: usize, unused_hint: usize);
}

#[cfg(target_arch = "aarch64")]
unsafe extern "C" {
    pub fn pryx_page_zva64() -> usize;
    pub fn pryx_page_clear_zva(dst: *mut u8, unused: *const u8, len: usize, unused_hint: usize);
    pub fn pryx_page_diff_neon(a: *const u8, b: *const u8, len: usize) -> usize;
    pub fn pryx_page_value_neon(a: *const u8, value: u8, len: usize) -> usize;
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
unsafe extern "C" {
    pub fn pryx_page_copy_nt(dst: *mut u8, src: *const u8, len: usize, prefetch: usize);
    pub fn pryx_page_clear_nt(dst: *mut u8, unused: *const u8, len: usize, unused_hint: usize);
    pub fn pryx_page_hash_lanes(src: *const u8, groups: usize, lanes: *mut u32);
}
