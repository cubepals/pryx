// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2
//! Run the page-specialization self-check or paired CI-only sparse-file experiment.
//! Parts (`src/`): `main.rs` command entry point, `sleep.rs` native paired samples.

use pryx_page_generator::{CopyKernel, CopyPlan, Kernel, Plan, copy_scalar, scalar};

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
            let from = [3, 7, 0, 5, 9, 11];
            let mut to = [0; 6];
            copy_scalar(&mut to, &from);
            assert_eq!(to, from);
            for plan in CopyPlan::choices() {
                let copy = CopyKernel::for_plan(plan).ok_or("host refused generated copy")?;
                let src: Vec<_> = (0..521).map(|i| (i % 251) as u8).collect();
                let mut dst = vec![0; src.len() + 5];
                for len in 0..=512 {
                    copy.copy(&mut dst[3..3 + len], &src[5..5 + len]);
                    assert_eq!(dst[3..3 + len], src[5..5 + len]);
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
            for plan in CopyPlan::choices() {
                let kernel = CopyKernel::for_plan(plan).ok_or("copy generation refused")?;
                let path = std::path::Path::new(dir).join(plan.slug()).with_extension("bin");
                std::fs::write(path, kernel.encoding().ok_or("missing finalized copy bytes")?)?;
            }
            eprintln!("L1 data cache (line bytes, capacity bytes): {:?}", pryx_page_generator::l1_data_cache());
            for bytes in [16, 64, 128, 4096, 16384, 65536, 2097152] {
                eprintln!("copy dispatch for {bytes} bytes: {:?}", CopyPlan::detected(bytes));
            }
            Ok(())
        }
        [mode, dir] if mode == "sleep" => pryx_lab_codegen::sample(dir, &[]),
        _ => Err("usage: pryx-codegen self-check | encodings NEW_DIRECTORY | sleep EXISTING_DIRECTORY".into()),
    }
}
