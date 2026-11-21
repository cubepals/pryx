// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Page operations for opt-in measurements. No production caller or persisted fingerprint.
//! Parts (`src/`): `lib.rs` defines slice contracts and composes runs, `reference.rs`
//! specifies the answers, `dispatch.rs` checks the host, `x86_64.rs` and `aarch64.rs`
//! implement the Rust vector candidates. `bulk.rs` dispatches stores and `fingerprint.rs`
//! compares non-cryptographic candidates with BLAKE3.
#![cfg(feature = "page-kernels")]

mod bulk;
mod dispatch;
mod fingerprint;
mod reference;
#[cfg(all(not(miri), target_os = "linux", target_arch = "x86_64"))]
#[path = "x86_64.rs"]
mod vector;
#[cfg(all(not(miri), target_os = "linux", target_arch = "aarch64", target_endian = "little"))]
#[path = "aarch64.rs"]
mod vector;

pub use bulk::{Store, clear, copy};
pub use dispatch::Backend;
use dispatch::Kind;
pub use fingerprint::{hash, verified_match};
use std::num::NonZeroUsize;

fn tail_compare(a: &[u8], b: Option<&[u8]>, value: u8) -> usize {
    match b {
        Some(b) => reference::mismatch(a, b),
        None => reference::value(a, value),
    }
}
fn compare(backend: Backend, a: &[u8], b: Option<&[u8]>, value: u8) -> usize {
    debug_assert!(b.is_none_or(|b| b.len() == a.len()));
    match backend.0 {
        Kind::Scalar => tail_compare(a, b, value),
        Kind::Library => match b {
            Some(b) if a == b => a.len(),
            Some(b) => reference::mismatch(a, b),
            None if value == 0 => {
                static BLANK: [u8; 2 << 20] = [0; 2 << 20];
                for (i, part) in a.chunks(BLANK.len()).enumerate() {
                    if part != &BLANK[..part.len()] {
                        return i * BLANK.len() + reference::value(part, 0);
                    }
                }
                a.len()
            }
            None => reference::value(a, value),
        },
        #[cfg(all(
            not(miri),
            target_os = "linux",
            target_endian = "little",
            any(target_arch = "x86_64", target_arch = "aarch64")
        ))]
        Kind::Vector => unsafe { vector::compare(a, b, value) },
        #[cfg(all(not(miri), target_os = "linux", target_arch = "x86_64"))]
        Kind::Avx2 => unsafe {
            match b {
                Some(b) => pryx_page_asm::pryx_page_diff_avx2(a.as_ptr(), b.as_ptr(), a.len()),
                None => pryx_page_asm::pryx_page_value_avx2(a.as_ptr(), value, a.len()),
            }
        },
        #[cfg(all(not(miri), target_os = "linux", target_arch = "x86_64"))]
        Kind::Avx512 => unsafe {
            match b {
                Some(b) => pryx_page_asm::pryx_page_diff_avx512(a.as_ptr(), b.as_ptr(), a.len()),
                None => pryx_page_asm::pryx_page_value_avx512(a.as_ptr(), value, a.len()),
            }
        },
        #[cfg(all(not(miri), target_os = "linux", target_arch = "aarch64", target_endian = "little"))]
        Kind::Neon => unsafe {
            match b {
                Some(b) => pryx_page_asm::pryx_page_diff_neon(a.as_ptr(), b.as_ptr(), a.len()),
                None => pryx_page_asm::pryx_page_value_neon(a.as_ptr(), value, a.len()),
            }
        },
    }
}

pub fn zero(backend: Backend, a: &[u8]) -> bool {
    compare(backend, a, None, 0) == a.len()
}

/// Empty and singleton slices are homogeneous.
pub fn same_value(backend: Backend, a: &[u8]) -> bool {
    if backend == Backend::LIBRARY {
        return a.len() < 2 || a[1..] == a[..a.len() - 1];
    }
    a.first().is_none_or(|&v| compare(backend, a, None, v) == a.len())
}
pub fn first_diff(backend: Backend, a: &[u8], b: &[u8]) -> Option<usize> {
    let shared = a.len().min(b.len());
    let offset = compare(backend, &a[..shared], Some(&b[..shared]), 0);
    (offset < shared || a.len() != b.len()).then_some(offset)
}
/// Visit maximal contiguous byte ranges occupied by zero pages.
pub fn runs(backend: Backend, bytes: &[u8], page: NonZeroUsize, mut visit: impl FnMut(std::ops::Range<usize>)) {
    let mut start = None;
    for (i, chunk) in bytes.chunks(page.get()).enumerate() {
        if zero(backend, chunk) {
            start.get_or_insert(i * page.get());
        } else if let Some(first) = start.take() {
            visit(first..i * page.get());
        }
    }
    if let Some(first) = start {
        visit(first..bytes.len());
    }
}

fn tail_transfer(dst: &mut [u8], src: Option<&[u8]>) {
    match src {
        Some(src) => dst.copy_from_slice(src),
        None => dst.fill(0),
    }
}
