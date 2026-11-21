// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Exercise every available backend, both end sentinels, and binary fill patterns through 8 bytes.
#![cfg(feature = "page-kernels")]
use pryx_page_kernels::*;
use std::num::NonZeroUsize;

fn check_pair(backend: Backend, a: &[u8], b: &[u8]) {
    let scalar = Backend::SCALAR;
    assert_eq!(zero(backend, a), zero(scalar, a), "{backend:?}");
    assert_eq!(same_value(backend, a), same_value(scalar, a));
    assert_eq!(first_diff(backend, a, b), first_diff(scalar, a, b));
    assert_eq!(verified_match(backend, a, b, 0, 0), a == b);
    if backend != Backend::LIBRARY {
        assert_eq!(hash(backend, a), hash(scalar, a));
    }
}
#[test]
fn exhaustive_small() {
    for backend in Backend::available() {
        for word in 0..=u16::MAX {
            let pair = word.to_le_bytes();
            check_pair(backend, &pair, &[pair[0], pair[0]]);
        }
        for len in 0..=8 {
            for mask in 0..(1 << len) {
                let a: Vec<_> = (0..len).map(|i| if mask & (1 << i) != 0 { 255 } else { 0 }).collect();
                check_pair(backend, &a, &vec![0; len]);
                check_pair(backend, &a, &a);
                check_pair(backend, &a, &[0; 9]);
            }
        }
    }
}
#[test]
fn aligned_and_unaligned_properties() {
    let mut a = [0u8; 768];
    let mut random = 7654321u32;
    for v in &mut a {
        random ^= random << 13;
        random ^= random >> 17;
        random ^= random << 5;
        *v = random as u8;
    }
    for backend in Backend::available() {
        for offset in 0..64 {
            for len in [0, 1, 7, 15, 16, 17, 31, 32, 33, 63, 64, 65, 127, 129, 257, 511] {
                let src = &a[offset..offset + len];
                check_pair(backend, src, src);
                for pos in [0, len / 2, len.saturating_sub(1)] {
                    let mut other = src.to_vec();
                    if let Some(v) = other.get_mut(pos) {
                        *v ^= 128;
                    }
                    check_pair(backend, src, &other);
                }
                for &store in Store::available(backend) {
                    let mut dst = [127; 768];
                    let start = 63 - offset;
                    let clip = &mut dst[start..start + len];
                    copy(backend, store, clip, src, 128);
                    assert_eq!(clip, src);
                    clear(backend, store, clip);
                    assert!(clip.iter().all(|&v| v == 0));
                    assert!(dst[..start].iter().chain(&dst[start + len..]).all(|&v| v == 127));
                }
            }
        }
        let mut actual = Vec::new();
        runs(backend, &[0, 0, 0, 0, 1, 0, 0], NonZeroUsize::new(2).unwrap(), |range| actual.push(range));
        assert_eq!(actual, [0..4, 6..7]);
    }
}
