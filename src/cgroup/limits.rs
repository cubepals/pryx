// SPDX-License-Identifier: FSL-1.1-ALv2
//! One sandbox cgroup's resource files: the memory, CPU and process limits written before
//! anything runs in it, freezing it whole, and what the kernel counted (usage, throttling, OOM
//! kills). The file names and formats are cgroup v2's (Documentation/admin-guide/cgroup-v2.rst).
//! Not which controllers the daemon has to give: that is `delegation.rs`.
use std::{fs, io, io::Write, path::Path};

/// cpu.max's default period in microseconds, as cgroup-v2.rst documents it (the same as cgroup
/// v1's cpu.cfs_period_us default); a quota is a share of it.
pub(crate) const CPU_PERIOD_MICROS: u64 = 100_000;

/// What the kernel enforces for one sandbox; `None` leaves that file as the kernel made it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Limits {
    /// memory.max, with no swap beyond it (memory.swap.max 0), as a container's limit has.
    pub(crate) memory_bytes: Option<u64>,
    /// cpu.max: the controller is on; `Some` is a ceiling in microseconds per period.
    pub(crate) cpu_quota_micros: Option<Option<u64>>,
    /// pids.max.
    pub(crate) pids: Option<u64>,
}

/// A ceiling of `nano_cpus` billionths of a CPU as cpu.max's quota per default period.
pub(crate) fn cpu_quota(nano_cpus: i64) -> Option<u64> {
    let nano_cpus = u64::try_from(nano_cpus).ok().filter(|value| *value > 0)?;
    let nanos_per_second = u64::try_from(std::time::Duration::from_secs(1).as_nanos()).ok()?;
    Some(nano_cpus.checked_mul(CPU_PERIOD_MICROS)? / nanos_per_second)
}

fn cpu_max(quota: Option<u64>) -> String {
    match quota {
        Some(quota) => format!("{quota} {CPU_PERIOD_MICROS}"),
        None => format!("max {CPU_PERIOD_MICROS}"),
    }
}

/// Writes `limits` into `cgroup`'s existing files, refusing to go on if any write fails: the
/// sandbox never runs with less than it asked for. A missing memory.swap.max means the kernel
/// accounts no swap, so there is none to forbid.
pub(crate) fn write(cgroup: &Path, limits: &Limits) -> io::Result<()> {
    let set = |name: &str, value: String| {
        fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(cgroup.join(name))
            .and_then(|mut file| file.write_all(value.as_bytes()))
            .map_err(|error| io::Error::new(error.kind(), format!("writing {value} to {name}: {error}")))
    };
    if let Some(bytes) = limits.memory_bytes {
        set("memory.max", bytes.to_string())?;
        if cgroup.join("memory.swap.max").exists() {
            set("memory.swap.max", "0".into())?;
        }
    }
    if let Some(quota) = limits.cpu_quota_micros {
        set("cpu.max", cpu_max(quota))?;
    }
    if let Some(pids) = limits.pids {
        set("pids.max", pids.to_string())?;
    }
    Ok(())
}

/// What `cgroup`'s files say now, read back to prove a write took: the kernel may round.
pub(crate) fn read(cgroup: &Path) -> io::Result<Limits> {
    let text = |name: &str| fs::read_to_string(cgroup.join(name)).map(|text| text.trim().to_owned());
    let number = |text: &str| if text == "max" { Some(None) } else { text.parse::<u64>().ok().map(Some) };
    let optional = |name: &str| match text(name) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    };
    let memory_bytes = optional("memory.max")?.as_deref().and_then(number).flatten();
    let cpu_quota_micros = optional("cpu.max")?.and_then(|text| {
        let (quota, period) = text.split_once(' ')?;
        (period.parse::<u64>().ok()? == CPU_PERIOD_MICROS).then(|| number(quota)).flatten()
    });
    let pids = optional("pids.max")?.as_deref().and_then(number).flatten();
    Ok(Limits { memory_bytes, cpu_quota_micros, pids })
}

/// Freezes or thaws every process in `cgroup` at once, then waits until the kernel says it is
/// done (cgroup.events `frozen`), so nothing forks past a freeze.
pub(crate) fn freeze(cgroup: &Path, frozen: bool) -> io::Result<()> {
    const WAIT: std::time::Duration = std::time::Duration::from_secs(5);
    fs::write(cgroup.join("cgroup.freeze"), if frozen { "1" } else { "0" })?;
    let deadline = std::time::Instant::now() + WAIT;
    while is_frozen(cgroup)? != frozen {
        if std::time::Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("cgroup {} did not {}", cgroup.display(), if frozen { "freeze" } else { "thaw" }),
            ));
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    Ok(())
}

/// Whether the kernel has frozen every process in `cgroup`.
pub(crate) fn is_frozen(cgroup: &Path) -> io::Result<bool> {
    Ok(field(&fs::read_to_string(cgroup.join("cgroup.events"))?, "frozen") == Some(1))
}

/// A `key value` line's value from a flat-keyed interface file (cpu.stat, memory.events, ...).
fn field(text: &str, key: &str) -> Option<u64> {
    text.lines().find_map(|line| line.strip_prefix(key)?.strip_prefix(' ')?.trim().parse().ok())
}

/// What the kernel counted for one cgroup; a field its controller doesn't give is `None`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Usage {
    pub(crate) memory_bytes: Option<u64>,
    pub(crate) inactive_file_bytes: Option<u64>,
    pub(crate) cpu_micros: Option<u64>,
    pub(crate) throttled_periods: Option<u64>,
    pub(crate) pids: Option<u64>,
    /// Processes the OOM killer killed in it (memory.events `oom_kill`).
    pub(crate) oom_kills: Option<u64>,
}

pub(crate) fn usage(cgroup: &Path) -> Usage {
    let text = |name: &str| fs::read_to_string(cgroup.join(name)).ok();
    let number = |name: &str| text(name).and_then(|text| text.trim().parse().ok());
    let cpu = text("cpu.stat");
    let memory = text("memory.stat");
    Usage {
        memory_bytes: number("memory.current"),
        inactive_file_bytes: memory.as_deref().and_then(|text| field(text, "inactive_file")),
        cpu_micros: cpu.as_deref().and_then(|text| field(text, "usage_usec")),
        throttled_periods: cpu.as_deref().and_then(|text| field(text, "nr_throttled")),
        pids: number("pids.current"),
        oom_kills: text("memory.events").as_deref().and_then(|text| field(text, "oom_kill")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ceiling_in_billionths_of_a_cpu_is_a_quota_of_the_default_period() {
        assert_eq!(cpu_quota(1_000_000_000), Some(CPU_PERIOD_MICROS));
        assert_eq!(cpu_quota(100_000_000), Some(10_000));
        assert_eq!(cpu_quota(2_500_000_000), Some(250_000));
        assert_eq!(cpu_quota(0), None);
        assert_eq!(cpu_quota(-1), None);
        assert_eq!(cpu_max(Some(10_000)), "10000 100000");
        assert_eq!(cpu_max(None), "max 100000");
    }

    #[test]
    fn limits_are_written_and_read_back_in_the_kernels_formats() {
        let cgroup = tempfile::tempdir().unwrap();
        for file in ["memory.max", "cpu.max", "pids.max"] {
            fs::write(cgroup.path().join(file), "max\n").unwrap();
        }
        let limits =
            Limits { memory_bytes: Some(256 * 1024 * 1024), cpu_quota_micros: Some(Some(10_000)), pids: Some(512) };
        // No memory.swap.max: a kernel that accounts no swap has none to forbid.
        write(cgroup.path(), &limits).unwrap();
        assert!(!cgroup.path().join("memory.swap.max").exists());
        assert_eq!(fs::read_to_string(cgroup.path().join("memory.max")).unwrap(), "268435456");
        assert_eq!(fs::read_to_string(cgroup.path().join("cpu.max")).unwrap(), "10000 100000");
        assert_eq!(fs::read_to_string(cgroup.path().join("pids.max")).unwrap(), "512");
        assert_eq!(read(cgroup.path()).unwrap(), limits);
        fs::write(cgroup.path().join("memory.swap.max"), "max\n").unwrap();
        fs::write(cgroup.path().join("cpu.max"), "max 100000\n").unwrap();
        write(cgroup.path(), &Limits { cpu_quota_micros: Some(None), ..limits }).unwrap();
        assert_eq!(fs::read_to_string(cgroup.path().join("memory.swap.max")).unwrap(), "0");
        assert_eq!(read(cgroup.path()).unwrap().cpu_quota_micros, Some(None));
    }

    #[test]
    fn a_failed_write_is_an_error_naming_the_file() {
        let missing = Path::new("/nonexistent-pryx-cgroup");
        let refused = write(missing, &Limits { pids: Some(64), ..Limits::default() }).unwrap_err();
        assert!(refused.to_string().contains("pids.max"), "{refused}");
    }

    #[test]
    fn the_kernels_counters_are_read_by_key() {
        let cgroup = tempfile::tempdir().unwrap();
        let write = |name: &str, text: &str| fs::write(cgroup.path().join(name), text).unwrap();
        write("memory.current", "1048576\n");
        write("memory.stat", "anon 4096\nfile 8192\ninactive_file 4096\nactive_file 4096\n");
        write(
            "cpu.stat",
            "usage_usec 2000\nuser_usec 1500\nsystem_usec 500\nnr_periods 20\nnr_throttled 7\nthrottled_usec 900\n",
        );
        write("pids.current", "3\n");
        write("memory.events", "low 0\nhigh 0\nmax 12\noom 1\noom_kill 1\noom_group_kill 0\n");
        write("cgroup.events", "populated 1\nfrozen 0\n");
        assert_eq!(
            usage(cgroup.path()),
            Usage {
                memory_bytes: Some(1_048_576),
                inactive_file_bytes: Some(4096),
                cpu_micros: Some(2000),
                throttled_periods: Some(7),
                pids: Some(3),
                oom_kills: Some(1),
            }
        );
        assert!(!is_frozen(cgroup.path()).unwrap());
        write("cgroup.events", "populated 1\nfrozen 1\n");
        assert!(is_frozen(cgroup.path()).unwrap());
        assert_eq!(usage(Path::new("/nonexistent-pryx-cgroup")), Usage::default());
    }

    /// The kernel holds a cgroup to what `write` wrote: past memory.max a process is OOM-killed
    /// inside it, past cpu.max it is throttled, and frozen it is killed whole. CI runs it as the
    /// only process of a transient service with Delegate=yes, as an ordinary user.
    #[test]
    #[ignore = "moves this process into its own leaf and runs processes under real limits"]
    fn the_kernel_holds_a_cgroup_to_its_limits() {
        use super::super::delegation::{CPU, MEMORY, delegate};
        use std::os::unix::process::ExitStatusExt;
        let delegated = delegate().unwrap_or_else(|problem| panic!("{}: {}", problem.evidence, problem.action));
        assert!(delegated.has(MEMORY) && delegated.has(CPU), "{delegated:?}");
        let cgroup = delegated.root.join(format!("pryx-limits-test-{}", std::process::id()));
        super::super::create_cgroup(&cgroup).unwrap();
        let limits = Limits { memory_bytes: Some(64 << 20), cpu_quota_micros: Some(Some(10_000)), pids: Some(64) };
        write(&cgroup, &limits).unwrap();
        assert_eq!(read(&cgroup).unwrap(), limits);
        let python = |code: &str| {
            let mut command = std::process::Command::new("python3");
            command.args(["-c", code]);
            super::super::join_on_exec(&mut command, &cgroup).unwrap();
            command
        };
        // Bytes filled, not only reserved, so every page is charged.
        let status = python("b = b'x' * (256 << 20)").status().unwrap();
        assert_eq!(status.signal(), Some(libc::SIGKILL), "{status}");
        assert!(usage(&cgroup).oom_kills.unwrap_or_default() >= 1, "{:?}", usage(&cgroup));
        let before = usage(&cgroup);
        let started = std::time::Instant::now();
        let busy = "import time\nend = time.monotonic() + 1\nwhile time.monotonic() < end: pass";
        assert!(python(busy).status().unwrap().success());
        let wall = u64::try_from(started.elapsed().as_micros()).unwrap();
        let after = usage(&cgroup);
        let used = after.cpu_micros.unwrap() - before.cpu_micros.unwrap();
        assert!(after.throttled_periods.unwrap() > before.throttled_periods.unwrap(), "{after:?}");
        // A tenth of a CPU, with room for the interpreter's start and each period's edges.
        assert!(used < wall / 4, "{used} µs of CPU in {wall} µs");
        let mut sleeper = python("import time\ntime.sleep(30)").spawn().unwrap();
        freeze(&cgroup, true).unwrap();
        super::super::fence_cgroup(&cgroup).unwrap();
        assert_eq!(sleeper.wait().unwrap().signal(), Some(libc::SIGKILL));
        super::super::wait_empty(&cgroup).unwrap();
        super::super::remove_cgroup(&cgroup).unwrap();
    }
}
