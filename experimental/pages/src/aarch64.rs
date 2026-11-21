// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Explicit Rust NEON candidates. Callers check Advanced SIMD before use.
use std::arch::aarch64::*;

#[target_feature(enable = "neon")]
pub unsafe fn compare(a: &[u8], b: Option<&[u8]>, value: u8) -> usize {
    let pattern = vdupq_n_u8(value);
    let full = a.len() / 16 * 16;
    for i in (0..full).step_by(16) {
        unsafe {
            let left = vld1q_u8(a.as_ptr().add(i));
            let right = b.map_or(pattern, |b| vld1q_u8(b.as_ptr().add(i)));
            if vmaxvq_u8(veorq_u8(left, right)) != 0 {
                return i + super::tail_compare(&a[i..i + 16], b.map(|b| &b[i..i + 16]), value);
            }
        }
    }
    full + super::tail_compare(&a[full..], b.map(|b| &b[full..]), value)
}

#[target_feature(enable = "neon")]
pub unsafe fn transfer(dst: &mut [u8], src: Option<&[u8]>) {
    let full = dst.len() / 16 * 16;
    for i in (0..full).step_by(16) {
        unsafe {
            let v = src.map_or_else(|| vdupq_n_u8(0), |s| vld1q_u8(s.as_ptr().add(i)));
            vst1q_u8(dst.as_mut_ptr().add(i), v);
        }
    }
    super::tail_transfer(&mut dst[full..], src.map(|s| &s[full..]));
}

#[target_feature(enable = "neon")]
pub unsafe fn lanes(bytes: &[u8], state: &mut [u32; 4]) {
    unsafe {
        let mut acc = vld1q_u32(state.as_ptr());
        for chunk in bytes.as_chunks::<16>().0 {
            acc = vmulq_n_u32(veorq_u32(acc, vreinterpretq_u32_u8(vld1q_u8(chunk.as_ptr()))), super::reference::PRIME);
        }
        vst1q_u32(state.as_mut_ptr(), acc);
    }
}
