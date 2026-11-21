// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2
//! Compare complete writers on the A1 resident image. Timing ends after data sync.

use pryx_page_generator::{CopyKernel, Kernel, Plan, scalar};
use std::{
    error::Error,
    fs::{self, File},
    io::{self, Read, Seek, SeekFrom, Write},
    os::unix::fs::MetadataExt,
    process::Command,
    time::Instant,
};

pub const PAGE: usize = 4096;
const MIB: usize = 1 << 20;
pub type Result<T> = std::result::Result<T, Box<dyn Error>>;
type WriteImage = dyn Fn(&mut File, &[u8]) -> Result<()>;

pub struct Baseline {
    pub name: String,
    pub generated: bool,
    pub write: Box<WriteImage>,
}

/// Keep one file offset, as A1 does. Adjacent dirty pages need no seek.
pub fn sparse(
    dst: &mut File,
    image: &[u8],
    skip: impl Fn(&[u8]) -> bool,
    mut write_page: impl FnMut(&mut File, &[u8]) -> io::Result<()>,
) -> io::Result<()> {
    let mut end = 0;
    for (index, page) in image.as_chunks::<PAGE>().0.iter().enumerate() {
        if !skip(page) {
            let offset = index * PAGE;
            if offset != end {
                dst.seek(SeekFrom::Start(offset as u64))?;
            }
            write_page(dst, page)?;
            end = offset + PAGE;
        }
    }
    Ok(())
}

pub fn scan_writer(dst: &mut File, image: &[u8], skip: impl Fn(&[u8]) -> bool) -> Result<()> {
    Ok(sparse(dst, image, skip, |file, page| file.write_all(page))?)
}

pub fn generated_stage(dst: &mut File, image: &[u8], skip: impl Fn(&[u8]) -> bool) -> Result<()> {
    let kernel = CopyKernel::generate(PAGE).ok_or("generated copy unavailable")?;
    let mut staging = vec![0; PAGE];
    Ok(sparse(dst, image, skip, |file, page| {
        kernel.copy(&mut staging, page);
        file.write_all(&staging)
    })?)
}

fn verify(file: &mut File, image: &[u8]) -> io::Result<()> {
    if file.metadata()?.len() != image.len() as u64 {
        return Err(io::Error::other("wrong image length"));
    }
    file.rewind()?;
    let mut buf = vec![0; MIB];
    for chunk in image.chunks(MIB) {
        file.read_exact(&mut buf[..chunk.len()])?;
        if &buf[..chunk.len()] != chunk {
            return Err(io::Error::other("restored page data differs"));
        }
    }
    Ok(())
}

fn first_cpu() -> Result<u32> {
    let status = fs::read_to_string("/proc/self/status")?;
    let allowed = status.lines().find_map(|line| line.strip_prefix("Cpus_allowed_list:")).ok_or("missing CPU list")?;
    let cpu = allowed.trim().split([',', '-']).next().ok_or("empty CPU list")?.parse::<u32>()?;
    let command = Command::new("taskset").args(["-pc", &cpu.to_string(), &std::process::id().to_string()]).output()?;
    if !command.status.success() {
        return Err(format!("setting writer affinity: {}", String::from_utf8_lossy(&command.stderr)).into());
    }
    Ok(cpu)
}

fn fixture() -> Vec<u8> {
    let mut bytes = vec![0; 256 * MIB];
    let mut state = 1u32;
    for chunk in bytes.as_chunks_mut::<PAGE>().0.iter_mut().step_by(10) {
        for byte in chunk {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            *byte = (state >> 24) as u8;
        }
    }
    bytes
}

pub fn sample(dir: &str, baselines: &[Baseline]) -> Result<()> {
    if std::env::var("GITHUB_REPOSITORY").as_deref() != Ok("cubepals/pryx")
        || std::env::var("GITHUB_ACTIONS").as_deref() != Ok("true")
    {
        return Err("collect timings in the repository native CI".into());
    }
    let cpu = first_cpu()?;
    let image = fixture();
    let mut builtins = vec![Baseline {
        name: "scalar-zero".into(),
        generated: false,
        write: Box::new(|dst, bytes| scan_writer(dst, bytes, scalar)),
    }];
    for plan in Plan::static_choices() {
        let kernel = Kernel::precompiled(plan).ok_or("static plan unavailable")?;
        builtins.push(Baseline {
            name: format!("linked-zero-{}", plan.slug()),
            generated: false,
            write: Box::new(move |dst, bytes| scan_writer(dst, bytes, |page| kernel.scan(page))),
        });
    }
    builtins.push(Baseline {
        name: "generated-zero".into(),
        generated: true,
        write: Box::new(|dst, bytes| {
            let plan = Plan::detected().ok_or("no native vector implementation")?;
            let kernel = Kernel::generate(plan).ok_or("runtime assembler unavailable")?;
            scan_writer(dst, bytes, |page| kernel.scan(page))
        }),
    });
    let mut methods = builtins.iter().chain(baselines).collect::<Vec<_>>();
    println!("pair,order,implementation,generated,cpu,logical_bytes,allocated_bytes,elapsed_ns");
    for round in 0..=15 {
        if round > 0 {
            methods.reverse();
        }
        for (order, method) in methods.iter().enumerate() {
            let mut file = tempfile::tempfile_in(dir)?;
            let start = Instant::now();
            (method.write)(&mut file, &image)?;
            file.set_len(image.len() as u64)?;
            file.sync_data()?;
            let elapsed = start.elapsed().as_nanos();
            let allocated = file.metadata()?.blocks() * 512;
            verify(&mut file, &image)?;
            if round > 0 {
                println!(
                    "{},{order},{},{},{cpu},{},{allocated},{elapsed}",
                    round - 1,
                    method.name,
                    method.generated,
                    image.len()
                );
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_copy_restores_adjacent_data_and_holes() -> Result<()> {
        if Plan::detected().is_none() {
            return Ok(());
        }
        let mut image = vec![0; 5 * PAGE];
        for (index, page) in image.as_chunks_mut::<PAGE>().0.iter_mut().enumerate() {
            if matches!(index, 1 | 2) {
                page.fill(index as u8);
            }
        }
        let mut file = tempfile::tempfile()?;
        generated_stage(&mut file, &image, scalar)?;
        file.set_len(image.len() as u64)?;
        verify(&mut file, &image)?;
        file.set_len(image.len() as u64 + 1)?;
        assert!(verify(&mut file, &image).is_err());
        Ok(())
    }
}
