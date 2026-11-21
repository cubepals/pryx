// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2
//! A scalar reference and an opt-in native implementation for bounded zero scans.
//! Parts (`src/`): `lib.rs` reference and dispatch, `main.rs` installed self-check.

pub fn scalar(bytes: &[u8]) -> bool {
    bytes.iter().all(|&byte| byte == 0)
}

pub fn all_zero(bytes: &[u8]) -> bool {
    #[cfg(feature = "experimental")]
    if let Some(result) = pryx_lab_machine::all_zero(bytes) {
        return result;
    }
    scalar(bytes)
}

/// Compare unaligned slices, zero runs and isolated nonzero bytes for a fuzzer input.
pub fn check_input(bytes: &[u8]) {
    assert_eq!(all_zero(bytes), scalar(bytes));
    let split = bytes.first().copied().unwrap_or(0) as usize % (bytes.len().saturating_add(1));
    let suffix = &bytes[split..];
    assert_eq!(all_zero(suffix), scalar(suffix));
    // Raw random inputs rarely contain long zero runs. Use input bytes as
    // positions and values in an otherwise-zero, equally long allocation.
    let mut sparse = vec![0; bytes.len()];
    if let Some((&value, positions)) = bytes.split_first() {
        for &position in positions.iter().take(4) {
            sparse[usize::from(position) % bytes.len()] = value;
        }
    }
    assert_eq!(all_zero(&sparse), scalar(&sparse));
}
