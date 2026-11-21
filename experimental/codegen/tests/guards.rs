// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2
//! Check all CPU-supported static and generated plans against the scalar predicate
//! with guard pages at both ends and random alignment, lengths and sparse contents.

#![cfg(all(target_os = "linux", feature = "experimental", not(miri)))]
use pryx_page_generator::{Kernel, Plan, scalar};

#[test]
fn sparse_fuzz_at_inaccessible_neighbors() {
    // The middle page alone is accessible for the lifetime of every input slice.
    unsafe {
        let size = usize::try_from(libc::sysconf(libc::_SC_PAGESIZE)).unwrap();
        let mapping =
            libc::mmap(std::ptr::null_mut(), 3 * size, libc::PROT_NONE, libc::MAP_PRIVATE | libc::MAP_ANONYMOUS, -1, 0);
        assert_ne!(mapping, libc::MAP_FAILED);
        let middle = mapping.cast::<u8>().add(size);
        assert_eq!(libc::mprotect(middle.cast(), size, libc::PROT_READ | libc::PROT_WRITE), 0);
        let page = std::slice::from_raw_parts_mut(middle, size);
        for plan in Plan::static_choices() {
            let generated = Kernel::generate(plan).expect("run this test on a host permitting generated code");
            let linked = Kernel::precompiled(plan).unwrap();
            let mut state: u32 = 0x8369e33f;
            for round in 0..8192 {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                let len = if round < 513 { round } else { state as usize % (size + 1) };
                for offset in [0, size - len] {
                    let slice = &mut page[offset..offset + len];
                    assert!(generated.scan(slice) && linked.scan(slice));
                    if len > 0 {
                        let pos = (state.rotate_left(9) as usize) % len;
                        slice[pos] = (state as u8) | 1;
                        assert_eq!(generated.scan(slice), scalar(slice));
                        assert_eq!(linked.scan(slice), scalar(slice));
                        slice[pos] = 0;
                    }
                }
            }
        }
        assert_eq!(libc::munmap(mapping, 3 * size), 0);
    }
}

#[test]
fn copy_at_two_sided_guard_pages() {
    use pryx_page_generator::{CopyKernel, CopyPlan};

    // Alternating inaccessible pages isolate both buffers, including from each other.
    unsafe {
        let size = usize::try_from(libc::sysconf(libc::_SC_PAGESIZE)).unwrap();
        let mem =
            libc::mmap(std::ptr::null_mut(), size * 5, libc::PROT_NONE, libc::MAP_PRIVATE | libc::MAP_ANONYMOUS, -1, 0);
        assert_ne!(mem, libc::MAP_FAILED);
        let source = mem.cast::<u8>().add(size);
        let destination = source.add(2 * size);
        for ptr in [source, destination] {
            assert_eq!(libc::mprotect(ptr.cast(), size, libc::PROT_READ | libc::PROT_WRITE), 0);
        }
        for i in 0..size {
            source.add(i).write(((i * 241 + i / 19 + 2) % 251) as u8);
        }
        assert_eq!(libc::mprotect(source.cast(), size, libc::PROT_READ), 0);
        let src = std::slice::from_raw_parts(source, size);
        let dst = std::slice::from_raw_parts_mut(destination, size);
        for plan in CopyPlan::choices() {
            let kernel = CopyKernel::for_plan(plan).expect("native copy generation required for this test");
            for len in (0..=512).chain((514..size).step_by(37)).chain([size - 1, size]) {
                for from in [0, size - len] {
                    for to in [0, size - len] {
                        dst.fill(0xfa);
                        kernel.copy(&mut dst[to..to + len], &src[from..from + len]);
                        assert_eq!(&dst[to..to + len], &src[from..from + len]);
                        assert!(dst[..to].iter().chain(&dst[to + len..]).all(|&b| b == 0xfa));
                    }
                }
            }
        }
        assert_eq!(libc::munmap(mem, size * 5), 0);
    }
}
