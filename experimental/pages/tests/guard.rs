// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Both operands and the destination end immediately before PROT_NONE.
//! Leading PROT_NONE and byte canaries also catch reads and writes before the mapping.
#![cfg(all(feature = "page-kernels", target_os = "linux", not(miri)))]
use pryx_page_kernels::*;
use std::ptr::NonNull;

struct Guard {
    ptr: NonNull<u8>,
    page: usize,
}
impl Guard {
    fn new() -> Self {
        let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        assert!(page > 0);
        let page = page as usize;
        let ptr = unsafe {
            libc::mmap(std::ptr::null_mut(), page * 3, libc::PROT_NONE, libc::MAP_PRIVATE | libc::MAP_ANONYMOUS, -1, 0)
        };
        assert_ne!(ptr, libc::MAP_FAILED);
        let ptr: NonNull<u8> = NonNull::new(ptr.cast()).unwrap();
        assert_eq!(
            unsafe { libc::mprotect(ptr.as_ptr().add(page).cast(), page, libc::PROT_READ | libc::PROT_WRITE) },
            0
        );
        Self { ptr, page }
    }
    fn page(&mut self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.ptr.as_ptr().add(self.page), self.page) }
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        assert_eq!(unsafe { libc::munmap(self.ptr.as_ptr().cast(), self.page * 3) }, 0);
    }
}
#[test]
fn inaccessible_neighbors() {
    #[cfg(target_arch = "aarch64")]
    unsafe {
        let cpu = libc::sched_getcpu();
        assert!(cpu >= 0);
        let mut mask: libc::cpu_set_t = std::mem::zeroed();
        libc::CPU_SET(cpu as usize, &mut mask);
        assert_eq!(libc::sched_setaffinity(0, std::mem::size_of_val(&mask), &mask), 0);
    }
    let mut a = Guard::new();
    let mut b = Guard::new();
    let mut out = Guard::new();
    let size = a.page;
    for backend in Backend::available() {
        for len in (0..=195).chain([size - 1, size]) {
            for start in [0, size - len] {
                a.page().fill(117);
                b.page().fill(61);
                let a = &mut a.page()[start..start + len];
                let b = &mut b.page()[start..start + len];
                a.fill(0);
                b.fill(0);
                assert!(zero(backend, a));
                assert!(same_value(backend, b));
                assert!(first_diff(backend, a, b).is_none());
                if let Some(v) = b.last_mut() {
                    *v = 1;
                }
                assert_eq!(first_diff(backend, a, b), len.checked_sub(1));
                if backend != Backend::LIBRARY {
                    assert_eq!(hash(backend, b), hash(Backend::SCALAR, b));
                }
                for &store in Store::for_clear(backend) {
                    let page = out.page();
                    page.fill(249);
                    clear(backend, store, &mut page[start..start + len]);
                    assert_eq!(&page[start..start + len], a);
                    assert!(page[..start].iter().chain(&page[start + len..]).all(|&v| v == 249));
                }
                for &store in Store::available(backend) {
                    let page = out.page();
                    page.fill(248);
                    let span = &mut page[start..start + len];
                    copy(backend, store, span, b, 64);
                    assert_eq!(span, b);
                    clear(backend, store, span);
                    assert_eq!(span, a);
                    assert!(page[..start].iter().chain(&page[start + len..]).all(|&v| v == 248));
                }
            }
        }
    }
}
