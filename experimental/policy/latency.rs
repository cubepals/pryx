// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2
//! One sample of raw syscall latency with or without the real network-off guest policy.
//! Filters apply only to this disposable process. No daemon, sandbox or socket is opened.

use std::{error::Error, io, time::Instant};

#[allow(dead_code)]
#[path = "../../agent/src/process_policy/filter.rs"]
mod filter;
#[allow(dead_code)]
#[path = "../../agent/src/syscall_abi.rs"]
mod syscall_abi;

#[derive(Clone, Copy)]
enum Call {
    Getpid,
    Bind,
}

impl Call {
    fn batch(self, count: u32, filtered: bool) -> Result<u128, Box<dyn Error>> {
        let expected = match self {
            Self::Getpid => std::process::id().into(),
            Self::Bind => -1,
        };
        let started = Instant::now();
        for _ in 0..count {
            // SAFETY: getpid takes no arguments. bind receives an invalid descriptor and
            // a null address with zero length. Neither operation reads or writes user data.
            let value = unsafe {
                match self {
                    Self::Getpid => libc::syscall(libc::SYS_getpid),
                    Self::Bind => libc::syscall(libc::SYS_bind, -1i32, std::ptr::null::<libc::sockaddr>(), 0u32),
                }
            };
            if value != expected {
                return Err(format!("unexpected syscall result {value}").into());
            }
        }
        // Read errno before formatting output or starting any further syscall batch.
        let errno = io::Error::last_os_error().raw_os_error();
        let elapsed = started.elapsed().as_nanos();
        if matches!(self, Self::Bind) && errno != Some(if filtered { libc::EPERM } else { libc::EBADF }) {
            return Err(format!("unexpected bind errno {errno:?}, check the runner's enclosing policy").into());
        }
        Ok(elapsed)
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let [mode, call, iterations] = args.as_slice() else {
        return Err("expected: host|process getpid|bind|cache COUNT".into());
    };
    let filtered = match mode.as_str() {
        "host" => false,
        "process" => true,
        _ => return Err("unknown policy".into()),
    };
    if filtered {
        filter::apply(false)?;
        let parent = rustix::process::getppid().ok_or("missing benchmark parent")?;
        filter::child(parent.as_raw_nonzero().get() as u32)?;
        filter::apply(true)?;
    }
    if call == "cache" {
        // CONFIG_SECCOMP_CACHE_DEBUG and permission to read this process's file are optional.
        match std::fs::read_to_string("/proc/self/seccomp_cache") {
            Ok(cache) => print!("{cache}"),
            Err(error) => println!("unavailable: {error}"),
        }
        return Ok(());
    }
    let call = match call.as_str() {
        "getpid" => Call::Getpid,
        "bind" => Call::Bind,
        _ => return Err("unknown syscall".into()),
    };
    let count: u32 = iterations.parse()?;
    if count == 0 {
        return Err("COUNT must be positive".into());
    }
    call.batch(4096, filtered)?;
    println!("{}", call.batch(count, filtered)?);
    Ok(())
}
