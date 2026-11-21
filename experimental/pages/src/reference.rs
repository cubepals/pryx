// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Scalar reference, including the exact fingerprint specification.
pub fn mismatch(a: &[u8], b: &[u8]) -> usize {
    a.iter().zip(b).position(|(a, b)| a != b).unwrap_or(a.len().min(b.len()))
}

pub fn value(a: &[u8], v: u8) -> usize {
    a.iter().position(|&byte| byte != v).unwrap_or(a.len())
}

pub const BASIS: u32 = 2166136261;
pub const PRIME: u32 = 16777619;

pub fn lanes(bytes: &[u8], state: &mut [u32; 4]) {
    for word_group in bytes.as_chunks::<16>().0 {
        for (lane, word) in state.iter_mut().zip(word_group.as_chunks::<4>().0) {
            *lane = (*lane ^ u32::from_le_bytes(*word)).wrapping_mul(PRIME);
        }
    }
}

pub fn finish(lanes: [u32; 4], tail: &[u8], len: usize) -> u64 {
    let mut acc = 14695981039346656037u64;
    for b in (len as u64)
        .to_le_bytes()
        .into_iter()
        .chain(lanes.into_iter().flat_map(u32::to_le_bytes))
        .chain(tail.iter().copied())
    {
        acc = (acc ^ u64::from(b)).wrapping_mul(1099511628211);
    }
    acc
}
