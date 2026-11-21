// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2
//! Check both sides of the native loop with inaccessible neighboring pages.

#![cfg(all(target_os = "linux", not(miri), feature = "experimental"))]

#[test]
fn first_and_last_accessible_byte() {
    // SAFETY: a private mapping is retained for the whole test; only the
    // central page becomes accessible and all references lie inside it.
    unsafe {
        let page = usize::try_from(libc::sysconf(libc::_SC_PAGESIZE)).unwrap();
        let total = 3 * page;
        let mapping =
            libc::mmap(std::ptr::null_mut(), total, libc::PROT_NONE, libc::MAP_PRIVATE | libc::MAP_ANONYMOUS, -1, 0);
        assert_ne!(mapping, libc::MAP_FAILED);
        let middle = mapping.cast::<u8>().add(page);
        assert_eq!(libc::mprotect(middle.cast(), page, libc::PROT_READ | libc::PROT_WRITE), 0);
        let bytes = std::slice::from_raw_parts_mut(middle, page);
        for len in (0..=96).chain(page - 32..=page) {
            for offset in [0, page - len] {
                let part = &mut bytes[offset..offset + len];
                assert!(pryx_lab_proof::all_zero(part));
                for pos in [0, len / 2, len.saturating_sub(1)] {
                    if let Some(byte) = part.get_mut(pos) {
                        *byte = 1;
                        assert!(!pryx_lab_proof::all_zero(part));
                        part[pos] = 0;
                    }
                }
            }
        }
        assert_eq!(libc::munmap(mapping, total), 0);
    }
}
