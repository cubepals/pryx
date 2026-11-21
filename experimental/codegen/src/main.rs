// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2
//! Run the page-specialization self-check or paired CI-only sparse-file experiment.
//! Parts (`src/`): `main.rs` command entry point, `sleep.rs` native paired samples.

use pryx_page_generator::{Kernel, Plan, scalar};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [mode] if mode == "self-check" => {
            assert!(scalar(&[]));
            assert!(scalar(&[0; 4096]));
            assert!(!scalar(&[0, 128]));
            if Plan::detected().is_none() {
                eprintln!("scalar fallback: host exposes no eligible assembly");
            }
            for plan in Plan::static_choices() {
                let generated = Kernel::generate(plan).ok_or("host refused generated page kernel")?;
                let linked = Kernel::precompiled(plan).unwrap();
                let mut storage = [0; 513];
                for start in 0..16 {
                    for len in 0..=384 {
                        let bytes = &mut storage[start..start + len];
                        assert!(generated.scan(bytes) && linked.scan(bytes));
                        for pos in [0, len / 2, len.saturating_sub(1)] {
                            if let Some(value) = bytes.get_mut(pos) {
                                *value = 0x80;
                                assert_eq!(generated.scan(bytes), scalar(bytes));
                                assert_eq!(linked.scan(bytes), scalar(bytes));
                                bytes[pos] = 0;
                            }
                        }
                    }
                }
            }
            Ok(())
        }
        [mode, dir] if mode == "encodings" => {
            std::fs::create_dir(dir)?;
            for plan in Plan::static_choices() {
                let kernel = Kernel::generate(plan).ok_or("generation refused")?;
                let path = std::path::Path::new(dir).join(plan.slug()).with_extension("bin");
                std::fs::write(path, kernel.encoding().ok_or("missing finalized bytes")?)?;
            }
            Ok(())
        }
        [mode, dir] if mode == "sleep" => pryx_lab_codegen::sample(dir, &[]),
        _ => Err("usage: pryx-codegen self-check | encodings NEW_DIRECTORY | sleep EXISTING_DIRECTORY".into()),
    }
}
