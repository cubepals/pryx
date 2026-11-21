// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2
//! Measure runtime generation amortized over one sparse sleep. Time writing and
//! syncing a new file; independently read and compare its full logical content.

use pryx_page_generator::{Kernel, Plan, scalar};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    time::Instant,
};

const PAGE: usize = 4096;
const MIB: usize = 1024 * 1024;

pub struct Baseline {
    pub name: String,
    pub scan: Box<dyn Fn(&[u8]) -> bool>,
}

fn once(
    dir: &str,
    image: &[u8],
    plan: Option<Plan>,
    generated: bool,
    baseline: Option<&Baseline>,
) -> Result<u128, Box<dyn std::error::Error>> {
    let mut file = tempfile::tempfile_in(dir)?;
    let start = Instant::now();
    let kernel = match plan {
        Some(_) if generated => {
            let detected = Plan::detected().ok_or("no native vector implementation")?;
            Some(Kernel::generate(detected).ok_or("runtime assembler unavailable")?)
        }
        Some(p) => Some(Kernel::precompiled(p).ok_or("static plan unavailable")?),
        None => None,
    };
    for (index, page) in image.as_chunks::<PAGE>().0.iter().enumerate() {
        let zero = if let Some(candidate) = &kernel {
            candidate.scan(page)
        } else if let Some(baseline) = baseline {
            (baseline.scan)(page)
        } else {
            scalar(page)
        };
        if !zero {
            file.seek(SeekFrom::Start((index * PAGE) as u64))?;
            file.write_all(page)?;
        }
    }
    file.set_len(image.len() as u64)?;
    file.sync_data()?;
    let elapsed = start.elapsed().as_nanos();
    file.rewind()?;
    verify(&mut file, image)?;
    Ok(elapsed)
}

fn verify(file: &mut File, image: &[u8]) -> std::io::Result<()> {
    let mut buf = vec![0; MIB];
    for chunk in image.chunks(MIB) {
        file.read_exact(&mut buf[..chunk.len()])?;
        if &buf[..chunk.len()] != chunk {
            return Err(std::io::Error::other("restored page data differs"));
        }
    }
    Ok(())
}

pub fn sample(dir: &str, baselines: &[Baseline]) -> Result<(), Box<dyn std::error::Error>> {
    let repo = std::env::var("GITHUB_REPOSITORY").unwrap_or_default();
    if repo != "cubepals/pryx" || std::env::var("GITHUB_ACTIONS").as_deref() != Ok("true") {
        return Err("collect timings in the repository native CI".into());
    }
    let mut image = vec![0_u8; 256 * MIB];
    for (index, page) in image.as_chunks_mut::<PAGE>().0.iter_mut().enumerate().step_by(10) {
        for (offset, byte) in page.iter_mut().enumerate() {
            *byte = ((offset + index) % 251 + 1) as u8;
        }
    }
    let selected = Plan::detected().ok_or("no native vector implementation")?;
    println!("pair,order,implementation,plan,elapsed_ns");
    let mut methods = vec![("scalar", None, false, None)];
    methods.extend(Plan::static_choices().into_iter().map(|p| ("static", Some(p), false, None)));
    methods.extend(baselines.iter().map(|b| (b.name.as_str(), None, false, Some(b))));
    methods.push(("generated", Some(selected), true, None));
    for pair in 0..15 {
        if pair > 0 {
            methods.reverse();
        }
        for (order, (method, plan, generated, baseline)) in methods.iter().enumerate() {
            let elapsed = once(dir, &image, *plan, *generated, *baseline)?;
            println!("{pair},{order},{method},\"{plan:?}\",{elapsed}");
        }
    }
    Ok(())
}
