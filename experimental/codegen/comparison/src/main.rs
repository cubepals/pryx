// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2
//! Compare steady-state resident-image writers against pinned A1 implementations.

use pryx_lab_codegen::{Baseline, PAGE, Result, generated_stage, sample, sample_split, scan_writer, sparse};
use pryx_page_kernels::{Backend, Store, clear, copy, first_diff, runs, same_value, zero};
use std::{
    fs::File,
    io::{Seek, SeekFrom, Write},
    num::NonZeroUsize,
};

fn baseline(name: String, write: impl Fn(&mut File, &[u8]) -> Result<()> + 'static) -> Baseline {
    Baseline { name, generated: false, write: Box::new(write) }
}

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let dir = args.next().ok_or("expected output directory")?;
    let mut methods = vec![baseline("a1-Dense-library".into(), |dst, bytes| Ok(dst.write_all(bytes)?))];
    for b in Backend::available() {
        let name = b.name();
        methods
            .push(baseline(format!("a1-Zero-{name}"), move |dst, bytes| scan_writer(dst, bytes, |page| zero(b, page))));
        methods.push(baseline(format!("a1-Fill-{name}"), move |dst, bytes| {
            scan_writer(dst, bytes, |page| page[0] == 0 && same_value(b, page))
        }));
        methods.push(baseline(format!("a1-Diff-{name}"), move |dst, bytes| {
            let blank = vec![0; PAGE];
            scan_writer(dst, bytes, |page| first_diff(b, page, &blank).is_none())
        }));
        if matches!(name, "library" | "asm-avx512-scan") {
            methods.push(baseline(format!("a1-Runs-{name}"), move |dst, bytes| {
                let mut end = 0;
                let mut saved = Ok(());
                runs(b, bytes, NonZeroUsize::new(PAGE).unwrap(), |skip| {
                    if saved.is_ok() {
                        saved = dst
                            .write_all(&bytes[end..skip.start])
                            .and_then(|()| dst.seek(SeekFrom::Start(skip.end as u64)).map(|_| ()));
                    }
                    end = skip.end;
                });
                saved?;
                Ok(dst.write_all(&bytes[end..])?)
            }));
        }
        if matches!(name, "asm-neon" | "asm-avx2" | "library") {
            for &store in Store::available(b) {
                let hints: &[usize] =
                    if name.starts_with("asm") && store == Store::Native { &[0, 128, 256, 512] } else { &[0] };
                for &hint in hints {
                    methods.push(baseline(format!("a1-StageCopy-{name}-{store:?}-pf{hint}"), move |dst, bytes| {
                        let mut staging = vec![0; PAGE];
                        Ok(sparse(
                            dst,
                            bytes,
                            |page| zero(Backend::LIBRARY, page),
                            |file, page| {
                                copy(b, store, &mut staging, page, hint);
                                file.write_all(&staging)
                            },
                        )?)
                    }));
                }
            }
        }
        if matches!(name, "asm-neon" | "library") {
            methods.push(baseline(format!("a1-StageClear-{name}-Native"), move |dst, bytes| {
                let mut staging = vec![0; PAGE];
                for page in bytes.as_chunks::<PAGE>().0 {
                    if zero(Backend::LIBRARY, page) {
                        clear(b, Store::Native, &mut staging)
                    } else {
                        staging.copy_from_slice(page)
                    }
                    dst.write_all(&staging)?;
                }
                Ok(())
            }));
        }
    }
    methods.push(Baseline {
        name: "generated-stage-copy".into(),
        generated: true,
        write: Box::new(|dst, bytes| generated_stage(dst, bytes, |page| zero(Backend::LIBRARY, page))),
    });
    match (args.next().as_deref(), args.next(), args.next()) {
        (None, None, None) => sample(&dir, &methods),
        (Some("--split"), Some(winner), None) => {
            sample_split(&dir, &methods, &winner, |page| zero(Backend::LIBRARY, page))
        }
        _ => Err("expected DIRECTORY [--split STATIC_WRITER]".into()),
    }
}
