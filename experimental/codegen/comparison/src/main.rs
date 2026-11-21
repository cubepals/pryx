// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2
//! Include every host-supported A1 scanner in the same timed sparse writer.

use pryx_lab_codegen::{Baseline, sample};
use pryx_page_kernels::{Backend, zero};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::env::args().nth(1).ok_or("expected output directory")?;
    let baselines = Backend::available().into_iter().map(|backend| Baseline {
        name: format!("a1-{}", backend.name()),
        scan: Box::new(move |page| zero(backend, page)),
    }).collect::<Vec<_>>();
    sample(&dir, &baselines)
}
