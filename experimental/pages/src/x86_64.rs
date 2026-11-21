// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Explicit Rust AVX2 candidates. Callers check AVX2 before entering these functions.
use std::arch::x86_64::*;

#[target_feature(enable = "avx2")]
pub unsafe fn compare(a: &[u8], b: Option<&[u8]>, value: u8) -> usize {
    let pattern = _mm256_set1_epi8(value as i8);
    let full = a.len() / 32 * 32;
    for i in (0..full).step_by(32) {
        unsafe {
            let left = _mm256_loadu_si256(a.as_ptr().add(i).cast());
            let right = b.map_or(pattern, |b| _mm256_loadu_si256(b.as_ptr().add(i).cast()));
            let bits = !_mm256_movemask_epi8(_mm256_cmpeq_epi8(left, right)) as u32;
            if bits != 0 {
                return i + bits.trailing_zeros() as usize;
            }
        }
    }
    full + super::tail_compare(&a[full..], b.map(|b| &b[full..]), value)
}

#[target_feature(enable = "avx2")]
pub unsafe fn transfer(dst: &mut [u8], src: Option<&[u8]>) {
    let full = dst.len() / 32 * 32;
    for i in (0..full).step_by(32) {
        unsafe {
            let v = src.map_or_else(|| _mm256_setzero_si256(), |s| _mm256_loadu_si256(s.as_ptr().add(i).cast()));
            _mm256_storeu_si256(dst.as_mut_ptr().add(i).cast(), v);
        }
    }
    super::tail_transfer(&mut dst[full..], src.map(|s| &s[full..]));
}

#[target_feature(enable = "avx2")]
pub unsafe fn lanes(bytes: &[u8], state: &mut [u32; 4]) {
    unsafe {
        let mut acc = _mm_loadu_si128(state.as_ptr().cast());
        let factor = _mm_set1_epi32(super::reference::PRIME as i32);
        for chunk in bytes.as_chunks::<16>().0 {
            acc = _mm_mullo_epi32(_mm_xor_si128(acc, _mm_loadu_si128(chunk.as_ptr().cast())), factor);
        }
        _mm_storeu_si128(state.as_mut_ptr().cast(), acc);
    }
}
