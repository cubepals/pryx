// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

//! A fingerprint only indexes possible duplicates. The caller verifies the bytes.
//! Scalar, vector and assembly compute the same four parallel word-lane hashes.
use super::{Backend, Kind, first_diff, reference};

/// BLAKE3 is an independent performance baseline. Its output is truncated for the same key width.
pub fn hash(backend: Backend, a: &[u8]) -> u64 {
    let mut lanes = [reference::BASIS; 4];
    match backend.0 {
        Kind::Scalar => reference::lanes(a, &mut lanes),
        Kind::Library => return u64::from_le_bytes(blake3::hash(a).as_bytes()[..8].try_into().unwrap()),
        #[cfg(all(
            not(miri),
            target_os = "linux",
            target_endian = "little",
            any(target_arch = "x86_64", target_arch = "aarch64")
        ))]
        Kind::Vector => unsafe { super::vector::lanes(a, &mut lanes) },
        #[cfg(all(
            not(miri),
            target_os = "linux",
            target_endian = "little",
            any(target_arch = "x86_64", target_arch = "aarch64")
        ))]
        _ => unsafe { pryx_page_asm::pryx_page_hash_lanes(a.as_ptr(), a.len() / 16, lanes.as_mut_ptr()) },
    }
    reference::finish(lanes, &a[a.len() / 16 * 16..], a.len())
}

pub fn verified_match(backend: Backend, a: &[u8], b: &[u8], hash_a: u64, hash_b: u64) -> bool {
    hash_a == hash_b && first_diff(backend, a, b).is_none()
}
