// SPDX-License-Identifier: FSL-1.1-ALv2
//! Compare fixed x86 short-copy methods on one pinned CPU, with L1-sized working sets.
//! The caller records the available fast-string features. Timings include the safe slice
//! wrapper and portable tails. Generate and verify all methods outside the timed loop.

#[cfg(target_arch = "x86_64")]
mod native {
    use pryx_page_generator::{CopyKernel, CopyPlan};
    use std::{ffi::c_void, hint::black_box, time::Instant};

    const MAX: usize = 4096;
    const LENGTHS: &[usize] = &[1, 4, 8, 15, 16, 17, 31, 32, 48, 63, 64, 65, 96, 127, 128, 129, 192, 256, 512, 4096];
    const COPIES: usize = 65_536;

    #[repr(align(64))]
    struct Buffer([u8; MAX + 64]);

    struct Lane {
        src: Buffer,
        dst: Buffer,
    }

    fn lanes(count: usize) -> Vec<Lane> {
        (0..count)
            .map(|index| Lane {
                src: Buffer(std::array::from_fn(|i| ((241 * i + i / 19 + index) % 251) as u8)),
                dst: Buffer([0xff; MAX + 64]),
            })
            .collect()
    }

    enum Method {
        Library,
        Emitted(CopyPlan, CopyKernel),
    }

    impl Method {
        fn fields(&self) -> (String, &'static str, usize) {
            match self {
                Self::Library => ("libc-memcpy".into(), "libc", 0),
                Self::Emitted(plan, _) => {
                    let group = plan.group_bytes();
                    (plan.slug(), if group == 1 { "rep" } else { "vector" }, group)
                }
            }
        }
    }

    fn copy_batch(
        lanes: &mut [Lane],
        len: usize,
        src_offset: usize,
        dst_offset: usize,
        copies: usize,
        copy: impl Fn(&mut [u8], &[u8]),
    ) {
        for _ in 0..copies / lanes.len() {
            for lane in &mut *lanes {
                let dst = &mut lane.dst.0[dst_offset..dst_offset + len];
                let src = &lane.src.0[src_offset..src_offset + len];
                // Even for a tail-only plan, each call must still copy exactly these bytes.
                copy(black_box(dst), black_box(src));
            }
        }
    }

    fn check(lanes: &[Lane], len: usize, src_offset: usize, dst_offset: usize) {
        for lane in lanes {
            let dst = &lane.dst.0;
            assert_eq!(&dst[dst_offset..dst_offset + len], &lane.src.0[src_offset..src_offset + len]);
            assert!(dst[..dst_offset].iter().chain(&dst[dst_offset + len..]).all(|&value| value == 0xff));
        }
    }

    fn execute(method: &Method, lanes: &mut [Lane], len: usize, src_offset: usize, dst_offset: usize, copies: usize) {
        match method {
            Method::Library => {
                // Keep the actual maintained libc function as an indirect call. LLVM may
                // otherwise inline memcpy and turn this into a compiler-copy baseline.
                let memcpy =
                    black_box(libc::memcpy as unsafe extern "C" fn(*mut c_void, *const c_void, usize) -> *mut c_void);
                copy_batch(lanes, len, src_offset, dst_offset, copies, |dst, src| {
                    // The source and destination are disjoint, equally sized slices.
                    unsafe { memcpy(dst.as_mut_ptr().cast(), src.as_ptr().cast(), src.len()) };
                });
            }
            Method::Emitted(_, kernel) => {
                copy_batch(lanes, len, src_offset, dst_offset, copies, |dst, src| kernel.copy(dst, src));
            }
        }
    }

    fn methods() -> Result<Vec<Method>, Box<dyn std::error::Error>> {
        let mut out = vec![Method::Library];
        for plan in CopyPlan::choices() {
            out.push(Method::Emitted(plan, CopyKernel::for_plan(plan).ok_or("copy generation refused")?));
        }
        Ok(out)
    }

    #[test]
    fn copy_shapes_restore_exact_bytes_without_timing() {
        let candidates = methods().unwrap();
        let mut buffers = lanes(2);
        for len in LENGTHS.iter().copied() {
            for (src_offset, dst_offset) in [(0, 0), (1, 3)] {
                for method in &candidates {
                    for lane in &mut buffers {
                        lane.dst.0.fill(0xff);
                    }
                    execute(method, &mut buffers, len, src_offset, dst_offset, 4);
                    check(&buffers, len, src_offset, dst_offset);
                }
            }
        }
    }

    pub fn sample() -> Result<(), Box<dyn std::error::Error>> {
        if std::env::var("GITHUB_REPOSITORY").as_deref() != Ok("cubepals/pryx")
            || std::env::var("GITHUB_ACTIONS").as_deref() != Ok("true")
        {
            return Err("short-copy timings require the repository native CI worker".into());
        }
        let mut allowed = std::mem::MaybeUninit::<libc::cpu_set_t>::uninit();
        let cpu = unsafe {
            if libc::sched_getaffinity(0, size_of::<libc::cpu_set_t>(), allowed.as_mut_ptr()) != 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            let allowed = allowed.assume_init();
            if libc::CPU_COUNT(&allowed) != 1 {
                return Err("pin this process to one permitted CPU".into());
            }
            libc::sched_getcpu()
        };
        if cpu < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let (erms, fsrm) = CopyPlan::string_features();
        let mut methods = methods()?;
        println!(
            "pair,order,cpu,erms,fsrm,length,src_offset,dst_offset,implementation,family,group_bytes,native_prefix_bytes,copies,elapsed_ns"
        );
        for &len in LENGTHS {
            // Eight distinct spans for short copies; bound the actually touched source
            // plus destination range below 16 KiB even at the 4 KiB reference point.
            let max_lanes = (8192 / (len + 64)).clamp(1, 8);
            let count = 1 << max_lanes.ilog2();
            let mut buffers = lanes(count);
            for (src_offset, dst_offset) in [(0, 0), (1, 3)] {
                for round in 0..=15 {
                    if round > 0 {
                        methods.reverse();
                    }
                    for (order, method) in methods.iter().enumerate() {
                        for lane in &mut buffers {
                            lane.dst.0.fill(0xff);
                        }
                        let start = Instant::now();
                        execute(method, &mut buffers, len, src_offset, dst_offset, COPIES);
                        let elapsed = start.elapsed().as_nanos();
                        check(&buffers, len, src_offset, dst_offset);
                        if round > 0 {
                            let (name, family, group) = method.fields();
                            let prefix = len.checked_div(group).unwrap_or(0) * group;
                            println!(
                                "{},{order},{cpu},{erms},{fsrm},{len},{src_offset},{dst_offset},{name},{family},{group},{prefix},{COPIES},{elapsed}",
                                round - 1
                            );
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(target_arch = "x86_64")]
    {
        native::sample()
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        Err("this cohort compares x86 fast-string instructions".into())
    }
}
