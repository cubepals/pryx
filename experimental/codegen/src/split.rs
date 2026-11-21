// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2
//! Measure fresh emission separately from repeated complete sleeps using the same RX handle.

use crate::sleep::{
    Baseline, PAGE, Result, builtin_writers, cached_stage, first_cpu, fixture, native_ci, scan_writer, write_and_verify,
};
use pryx_page_generator::{CopyKernel, Kernel, Plan};
use std::{fs::File, time::Instant};

#[derive(Clone, Copy)]
enum Candidate {
    Zero,
    Copy,
}

enum Writer {
    Zero(Kernel),
    Copy(CopyKernel),
}

impl Candidate {
    fn name(self) -> &'static str {
        match self {
            Self::Zero => "generated-zero",
            Self::Copy => "generated-stage-copy",
        }
    }

    fn generate(self) -> Result<Writer> {
        match self {
            Self::Zero => {
                let plan = Plan::detected().ok_or("no native vector implementation")?;
                Ok(Writer::Zero(Kernel::generate(plan).ok_or("runtime assembler unavailable")?))
            }
            Self::Copy => Ok(Writer::Copy(CopyKernel::generate(PAGE).ok_or("generated copy unavailable")?)),
        }
    }
}

impl Writer {
    fn bytes(&self) -> Result<&[u8]> {
        match self {
            Self::Zero(kernel) => kernel.encoding(),
            Self::Copy(kernel) => kernel.encoding(),
        }
        .filter(|bytes| !bytes.is_empty())
        .ok_or_else(|| "finalized kernel has no exported bytes".into())
    }

    fn write(&self, dst: &mut File, image: &[u8], library_zero: fn(&[u8]) -> bool) -> Result<()> {
        match self {
            Self::Zero(kernel) => scan_writer(dst, image, |page| kernel.scan(page)),
            Self::Copy(kernel) => cached_stage(kernel, dst, image, library_zero),
        }
    }
}

pub fn sample_split(dir: &str, baselines: &[Baseline], strongest: &str, library_zero: fn(&[u8]) -> bool) -> Result<()> {
    native_ci()?;
    let cpu = first_cpu()?;
    let image = fixture();
    let builtins = builtin_writers()?;
    let control = builtins
        .iter()
        .chain(baselines)
        .find(|m| !m.generated && m.name == strongest)
        .ok_or("static winner of inclusive cohort is missing")?;

    let mut writers = [(Candidate::Zero, Candidate::Zero.generate()?), (Candidate::Copy, Candidate::Copy.generate()?)];
    println!(
        "pair,candidate,static_writer,order,cpu,logical_bytes,generation_ns,cached_writer_ns,static_writer_ns,cached_allocated_bytes,static_allocated_bytes"
    );
    for round in 0..=15 {
        // Round zero warms both complete writers; round one (pair zero) starts with the control.
        if round != 0 {
            writers.reverse();
        }
        for (candidate, cached) in &writers {
            let start = Instant::now();
            let fresh = candidate.generate()?;
            let generation_ns = start.elapsed().as_nanos();
            if fresh.bytes()? != cached.bytes()? {
                return Err("fresh code differs from the cached native kernel".into());
            }
            drop(fresh);
            let cached_trial = || write_and_verify(dir, &image, |dst, image| cached.write(dst, image, library_zero));
            let control_trial = || write_and_verify(dir, &image, &control.write);
            let static_first = round % 2 == 1;
            let (static_time, cached_time) = if static_first {
                let value = control_trial()?;
                (value, cached_trial()?)
            } else {
                let value = cached_trial()?;
                (control_trial()?, value)
            };
            if round > 0 {
                let order = if static_first { "static-first" } else { "cached-first" };
                println!(
                    "{},{},{strongest},{order},{cpu},{},{generation_ns},{},{},{},{}",
                    round - 1,
                    candidate.name(),
                    image.len(),
                    cached_time.0,
                    static_time.0,
                    cached_time.1,
                    static_time.1
                );
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sleep::verify;
    use pryx_page_generator::scalar;

    #[test]
    fn cached_handles_restore_repeated_images_with_identical_finalized_bytes() -> Result<()> {
        if Plan::detected().is_none() {
            return Ok(());
        }
        for candidate in [Candidate::Zero, Candidate::Copy] {
            let cached = candidate.generate()?;
            assert_eq!(cached.bytes()?, candidate.generate()?.bytes()?);
            let mut image = vec![0; PAGE * 4];
            for page_index in [1, 2, 0] {
                image[page_index * PAGE + 7] ^= 0x80;
                let mut file = tempfile::tempfile()?;
                cached.write(&mut file, &image, scalar)?;
                file.set_len(image.len() as u64)?;
                verify(&mut file, &image)?;
            }
        }
        Ok(())
    }
}
