// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

#![no_main]
use libfuzzer_sys::fuzz_target;
use pryx_page_kernels::*;
use std::num::NonZeroUsize;

fuzz_target!(|input: &[u8]| {
    let Some((&selector, input)) = input.split_first() else { return };
    let a = &input[..input.len().saturating_sub((selector as usize) % 3)];
    let mut b = input.to_vec();
    if !b.is_empty() {
        b[(usize::from(selector) * 257) % input.len()] ^= selector;
    }
    let page = NonZeroUsize::new((selector as usize) + 1).unwrap();
    let mut expected = Vec::new();
    runs(Backend::SCALAR, a, page, |r| expected.push(r));
    for backend in Backend::available() {
        assert_eq!(zero(backend, a), zero(Backend::SCALAR, a));
        assert_eq!(same_value(backend, a), same_value(Backend::SCALAR, a));
        assert_eq!(first_diff(backend, a, &b), first_diff(Backend::SCALAR, a, &b));
        if backend != Backend::LIBRARY {
            assert_eq!(hash(backend, a), hash(Backend::SCALAR, a));
        }
        let mut observed = Vec::new();
        runs(backend, a, page, |r| observed.push(r));
        assert_eq!(observed, expected);
        let mut scratch = vec![23; input.len() + 64];
        let start = selector as usize % 64;
        for &store in Store::available(backend) {
            copy(backend, store, &mut scratch[start..start + a.len()], a, usize::from(selector) * 16);
            assert_eq!(&scratch[start..start + a.len()], a);
        }
        for &store in Store::for_clear(backend) {
            scratch.fill(23);
            clear(backend, store, &mut scratch[start..start + a.len()]);
            assert!(scratch[start..start + a.len()].iter().all(|b| *b == 0));
            assert!(scratch[..start].iter().chain(&scratch[start + a.len()..]).all(|b| *b == 23));
        }
    }
});
