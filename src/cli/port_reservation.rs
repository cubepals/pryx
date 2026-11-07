// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Host installation reserves the configured workload range from outgoing ephemeral allocation.
//! Existing reservations are preserved. This is an operator action, never a sandbox privilege.
use std::{collections::BTreeSet, fs, path::Path};

use anyhow::{Context, bail, ensure};

// Linux networking/ip-sysctl.html: ip_local_reserved_ports is a comma-separated range list
// excluded from automatic TCP/UDP source-port assignment, independently of ip_local_port_range.
const KERNEL: &str = "proc/sys/net/ipv4/ip_local_reserved_ports";
const PERSISTED: &str = "etc/sysctl.d/99-zz-pryxd-ports.conf";

pub fn reserve(range: [u16; 2], root: &Path) -> anyhow::Result<()> {
    if root == Path::new("/") && !rustix::process::geteuid().is_root() {
        bail!("Reserving host ports requires root");
    }
    let kernel = root.join(KERNEL);
    let current = fs::read_to_string(&kernel).context("reading host ephemeral port reservations")?;
    let reserved = merge(&current, range)?;
    let target = root.join(PERSISTED);
    let contents = format!(
        "# Workload ports, preserving the host's existing reservations.\nnet.ipv4.ip_local_reserved_ports = {reserved}\n"
    );
    fs::create_dir_all(target.parent().expect("sysctl directory"))?;
    if fs::read_to_string(&target).ok().as_deref() != Some(&contents) {
        crate::durable::write_atomic(&target, contents.as_bytes(), 0o644)?;
    }
    if current.trim() != reserved {
        fs::write(kernel, &reserved).context("reserving workload ports from outgoing ephemeral allocation")?;
    }
    Ok(())
}

fn merge(current: &str, range: [u16; 2]) -> anyhow::Result<String> {
    ensure!(range[0] <= range[1], "Invalid workload port range");
    let mut ports = BTreeSet::new();
    for item in current.trim().split(',').filter(|s| !s.is_empty()) {
        let (low, high) = item.trim().split_once('-').unwrap_or((item.trim(), item.trim()));
        let (low, high) = (low.parse::<u16>()?, high.parse::<u16>()?);
        ensure!(low <= high, "Invalid existing port reservation");
        ports.extend(low..=high);
    }
    ports.extend(range[0]..=range[1]);
    let mut ranges: Vec<(u16, u16)> = Vec::new();
    for port in ports {
        if let Some((_, end)) = ranges.last_mut()
            && end.checked_add(1) == Some(port)
        {
            *end = port;
        } else {
            ranges.push((port, port));
        }
    }
    Ok(ranges
        .into_iter()
        .map(|(low, high)| if low == high { low.to_string() } else { format!("{low}-{high}") })
        .collect::<Vec<_>>()
        .join(","))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reservations_merge_without_losing_other_services() {
        assert_eq!(merge("10000-10010,42005,50000\n", [42000, 42999]).unwrap(), "10000-10010,42000-42999,50000");
        assert_eq!(merge("1,2-4,65535", [4, 6]).unwrap(), "1-6,65535");
        for invalid in ["2-1", "65536", "not-a-port"] {
            assert!(merge(invalid, [42000, 42999]).is_err());
        }
    }

    #[test]
    fn installation_updates_kernel_and_persistent_policy_idempotently() {
        let root = tempfile::tempdir().unwrap();
        let kernel = root.path().join(KERNEL);
        fs::create_dir_all(kernel.parent().unwrap()).unwrap();
        fs::write(&kernel, "12345,50000\n").unwrap();
        reserve([42000, 42999], root.path()).unwrap();
        assert_eq!(fs::read_to_string(&kernel).unwrap(), "12345,42000-42999,50000");
        let persisted = root.path().join(PERSISTED);
        let first = fs::read_to_string(&persisted).unwrap();
        assert!(first.contains("net.ipv4.ip_local_reserved_ports = 12345,42000-42999,50000"));
        reserve([42000, 42999], root.path()).unwrap();
        assert_eq!(fs::read_to_string(persisted).unwrap(), first);
    }
}
