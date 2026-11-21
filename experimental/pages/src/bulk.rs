// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Bulk copy and clear use disjoint slices. Explicit x86 string and integer-store
//! comparators apply only to native x86 assembly backends.
use super::{Backend, Kind, tail_transfer};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Store {
    Native,
    #[cfg(all(not(miri), target_os = "linux", target_endian = "little", target_arch = "aarch64"))]
    Zva64,
    #[cfg(all(not(miri), target_os = "linux", target_arch = "x86_64"))]
    IntegerNt,
    #[cfg(all(not(miri), target_os = "linux", target_arch = "x86_64"))]
    Rep,
}
impl Store {
    pub fn for_clear(backend: Backend) -> &'static [Store] {
        #[cfg(all(not(miri), target_os = "linux", target_endian = "little", target_arch = "aarch64"))]
        if matches!(backend.0, Kind::Neon) && unsafe { pryx_page_asm::pryx_page_zva64() } != 0 {
            return &[Self::Native, Self::Zva64];
        }
        Self::available(backend)
    }

    pub fn available(backend: Backend) -> &'static [Store] {
        #[cfg(all(not(miri), target_os = "linux", target_arch = "x86_64"))]
        if matches!(backend.0, Kind::Avx2 | Kind::Avx512) {
            return &[Self::Native, Self::IntegerNt, Self::Rep];
        }
        let _ = backend;
        &[Self::Native]
    }
}
pub fn copy(backend: Backend, store: Store, dst: &mut [u8], src: &[u8], prefetch: usize) {
    assert_eq!(dst.len(), src.len());
    transfer(backend, store, dst, Some(src), prefetch);
}
pub fn clear(backend: Backend, store: Store, dst: &mut [u8]) {
    transfer(backend, store, dst, None, 0);
}
fn transfer(backend: Backend, store: Store, dst: &mut [u8], src: Option<&[u8]>, prefetch: usize) {
    match backend.0 {
        Kind::Scalar => match src {
            Some(s) => {
                for (d, s) in dst.iter_mut().zip(s) {
                    *d = *s;
                }
            }
            None => {
                for d in dst {
                    *d = 0;
                }
            }
        },
        Kind::Library => tail_transfer(dst, src),
        #[cfg(all(
            not(miri),
            target_os = "linux",
            target_endian = "little",
            any(target_arch = "x86_64", target_arch = "aarch64")
        ))]
        Kind::Vector => unsafe { super::vector::transfer(dst, src) },
        #[cfg(all(
            not(miri),
            target_os = "linux",
            target_endian = "little",
            any(target_arch = "x86_64", target_arch = "aarch64")
        ))]
        _ => {
            use pryx_page_asm as asm;
            let routine = match (store, src.is_some()) {
                (Store::Native, true) => asm::pryx_page_copy_nt,
                (Store::Native, false) => asm::pryx_page_clear_nt,
                #[cfg(target_arch = "aarch64")]
                (Store::Zva64, false) => asm::pryx_page_clear_zva,
                #[cfg(target_arch = "aarch64")]
                (Store::Zva64, true) => panic!("DC ZVA only clears memory"),
                #[cfg(target_arch = "x86_64")]
                (Store::IntegerNt, true) => asm::pryx_page_copy_gpr,
                #[cfg(target_arch = "x86_64")]
                (Store::IntegerNt, false) => asm::pryx_page_clear_gpr,
                #[cfg(target_arch = "x86_64")]
                (Store::Rep, true) => asm::pryx_page_copy_rep,
                #[cfg(target_arch = "x86_64")]
                (Store::Rep, false) => asm::pryx_page_clear_rep,
            };
            unsafe { routine(dst.as_mut_ptr(), src.map_or(std::ptr::null(), |s| s.as_ptr()), dst.len(), prefetch) };
        }
    }
    let _ = (store, prefetch);
}
