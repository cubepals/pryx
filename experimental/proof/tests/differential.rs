// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2
//! Exhaust small byte strings, all singleton positions across vector tails,
//! and reproducible random inputs. Native bounds checks live in guard_pages.rs.

use pryx_lab_proof::{all_zero, check_input, scalar};

#[test]
fn empty_and_all_one_and_two_byte_values() {
    assert!(all_zero(&[]));
    let upper = if cfg!(miri) { 3 } else { u8::MAX };
    for a in 0..=upper {
        assert_eq!(all_zero(&[a]), a == 0);
        for b in 0..=upper {
            assert_eq!(all_zero(&[a, b]), a == 0 && b == 0);
        }
    }
}

#[test]
fn unaligned_starts_tails_and_every_nonzero_position() {
    let mut storage = [0u8; 128];
    let limit = if cfg!(miri) { 4 } else { 80 };
    for start in 0..16 {
        for length in 0..=limit {
            let bytes = &mut storage[start..start + length];
            assert!(all_zero(bytes));
            for pos in 0..length {
                bytes[pos] = 0x80;
                assert_eq!(all_zero(bytes), scalar(bytes));
                bytes[pos] = 0;
            }
        }
    }
}

#[test]
fn seeded_differential_corpus() {
    let mut state = 0xfa34_9621_u32;
    let mut draw = || {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        state
    };
    for _ in 0..if cfg!(miri) { 8 } else { 4096 } {
        let len = draw() as usize % if cfg!(miri) { 32 } else { 8193 };
        let input: Vec<_> = (0..len).map(|_| draw() as u8).collect();
        check_input(&input);
    }
}

#[test]
#[cfg(all(
    feature = "experimental",
    target_os = "linux",
    not(miri),
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
fn supported_native_instructions_reach_the_assembly() {
    #[cfg(target_arch = "x86_64")]
    let available = std::is_x86_feature_detected!("sse2");
    #[cfg(target_arch = "aarch64")]
    let available = std::arch::is_aarch64_feature_detected!("neon");
    if available {
        let mut bytes = [0; 33];
        assert_eq!(pryx_lab_machine::all_zero(&bytes[1..]), Some(true));
        bytes[16] = 8;
        assert_eq!(pryx_lab_machine::all_zero(&bytes[1..]), Some(false));
    }
}
