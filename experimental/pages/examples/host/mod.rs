// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Linux worker affinity and backing-page metadata for native timings.
pub fn pin(worker: usize) {
    unsafe {
        let mut mask: libc::cpu_set_t = std::mem::zeroed();
        assert_eq!(libc::sched_getaffinity(0, std::mem::size_of_val(&mask), &mut mask), 0);
        let cpu = (0..libc::CPU_SETSIZE as usize)
            .filter(|&i| libc::CPU_ISSET(i, &mask))
            .nth(worker)
            .expect("fewer allowed CPUs than workers");
        libc::CPU_ZERO(&mut mask);
        libc::CPU_SET(cpu, &mut mask);
        assert_eq!(libc::sched_setaffinity(0, std::mem::size_of_val(&mask), &mask), 0);
    }
}

pub fn backing(bytes: &[u8]) -> String {
    let addr = bytes.as_ptr() as usize;
    let smaps = std::fs::read_to_string("/proc/self/smaps").expect("read mappings");
    let mut selected = false;
    let mut fields = vec![];
    for line in smaps.lines() {
        let Some(first) = line.split_ascii_whitespace().next() else { continue };
        if let Some((a, b)) = first.split_once('-')
            && let (Ok(a), Ok(b)) = (usize::from_str_radix(a, 16), usize::from_str_radix(b, 16))
        {
            if selected {
                break;
            }
            selected = (a..b).contains(&addr);
        }
        if selected && matches!(first, "KernelPageSize:" | "MMUPageSize:" | "AnonHugePages:" | "THPeligible:") {
            fields.push(line.split_whitespace().collect::<Vec<_>>().join(" "));
        }
    }
    fields.join(", ")
}
