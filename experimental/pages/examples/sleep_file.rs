// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Local fixture of a guest-memory file. Retain raw elapsed times and allocated blocks.
//! Each result includes data sync and is validated against every original byte after timing.
//! This measures the file writer in isolation from monitor snapshot and VM state transitions.
mod host;
use pryx_page_kernels::*;
use std::{
    fs::{File, OpenOptions},
    io::{self, ErrorKind, Read, Seek, SeekFrom, Write},
    num::NonZeroUsize,
    os::{fd::AsRawFd, unix::fs::MetadataExt},
    path::Path,
    time::Instant,
};

fn extents(src: &mut File, dst: &mut File, len: usize) -> io::Result<()> {
    let mut start = 0;
    while start < len as i64 {
        let data = unsafe { libc::lseek(src.as_raw_fd(), start, libc::SEEK_DATA) };
        if data == -1 {
            let e = io::Error::last_os_error();
            if e.raw_os_error() == Some(libc::ENXIO) {
                break;
            }
            return Err(e);
        }
        let hole = unsafe { libc::lseek(src.as_raw_fd(), data, libc::SEEK_HOLE) };
        if hole == -1 {
            return Err(io::Error::last_os_error());
        }
        if data < start || hole <= data || hole > len as i64 {
            return Err(io::Error::other("invalid extent"));
        }
        src.seek(SeekFrom::Start(data as u64))?;
        dst.seek(SeekFrom::Start(data as u64))?;
        let count = (hole - data) as u64;
        let copied = io::copy(&mut (&mut *src).take(count), dst)?;
        if copied != count {
            return Err(ErrorKind::UnexpectedEof.into());
        }
        start = hole;
    }
    Ok(())
}
#[derive(Clone, Copy, Debug, PartialEq)]
enum Method {
    Dense,
    ExtentsAllocated,
    ExtentsSparse,
    Zero,
    Runs,
    Diff,
    Fill,
    Hash,
    StageCopy,
    StageClear,
}
fn write_pages(
    method: Method,
    b: Backend,
    dst: &mut File,
    bytes: &[u8],
    page: usize,
    store: Store,
    prefetch: usize,
) -> io::Result<()> {
    if method == Method::Dense {
        return dst.write_all(bytes);
    }
    if method == Method::Runs {
        let mut end = 0;
        let mut saved = Ok(());
        runs(b, bytes, NonZeroUsize::new(page).unwrap(), |skip| {
            if saved.is_ok() {
                saved = dst
                    .write_all(&bytes[end..skip.start])
                    .and_then(|_| dst.seek(SeekFrom::Start(skip.end as u64)).map(|_| ()));
            }
            end = skip.end;
        });
        saved?;
        return dst.write_all(&bytes[end..]);
    }
    let blank = vec![0; page];
    let zero_hash = if method == Method::Hash { hash(b, &blank) } else { 0 };
    let mut staging = vec![0; page];
    let mut position = 0;
    for (index, chunk) in bytes.chunks_exact(page).enumerate() {
        let skip = match method {
            Method::Zero => zero(b, chunk),
            Method::Diff => first_diff(b, chunk, &blank).is_none(),
            Method::Fill => chunk[0] == 0 && same_value(b, chunk),
            Method::Hash => verified_match(b, chunk, &blank, hash(b, chunk), zero_hash),
            // Keep sparse detection fixed for the transfer-only comparison.
            Method::StageCopy | Method::StageClear => zero(Backend::LIBRARY, chunk),
            _ => unreachable!(),
        };
        if method == Method::StageClear {
            // Dense staging of a decoded page. Its comparator is another dense staging writer.
            if skip {
                clear(b, store, &mut staging)
            } else {
                staging.copy_from_slice(chunk)
            }
            dst.write_all(&staging)?;
        } else if !skip {
            if position != index * page {
                dst.seek(SeekFrom::Start((index * page) as u64))?;
            }
            if method == Method::StageCopy {
                copy(b, store, &mut staging, chunk, prefetch);
                dst.write_all(&staging)?;
            } else {
                dst.write_all(chunk)?;
            }
            position = (index + 1) * page;
        }
    }
    Ok(())
}
fn new_file(path: &Path) -> io::Result<File> {
    OpenOptions::new().read(true).write(true).create_new(true).open(path)
}
fn verify(file: &mut File, bytes: &[u8]) -> io::Result<()> {
    if file.metadata()?.len() != bytes.len() as u64 {
        return Err(io::Error::other("wrong file length"));
    }
    file.rewind()?;
    let mut buf = vec![0; 1 << 20];
    for chunk in bytes.chunks(buf.len()) {
        file.read_exact(&mut buf[..chunk.len()])?;
        if &buf[..chunk.len()] != chunk {
            return Err(io::Error::other("wrong guest bytes"));
        }
    }
    Ok(())
}
fn main() -> io::Result<()> {
    host::pin(0);
    let args: Vec<_> = std::env::args().skip(1).collect();
    let [dir, page, mb, rounds] = args.as_slice() else { panic!("NEW_DIRECTORY PAGE_BYTES MIB ROUNDS") };
    let dir = Path::new(dir);
    std::fs::create_dir(dir)?;
    let page = page.parse::<usize>().unwrap();
    let size = mb.parse::<usize>().unwrap().checked_mul(1 << 20).unwrap();
    let rounds = rounds.parse::<usize>().unwrap();
    assert!(page > 0 && size > 0 && size.is_multiple_of(page) && rounds > 0);
    let mut bytes = vec![0; size];
    let mut state = 1u32;
    for (i, chunk) in bytes.chunks_exact_mut(page).enumerate() {
        if i % 10 != 0 {
            continue;
        }
        for v in chunk {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            *v = (state >> 24) as u8;
        }
    }
    eprintln!("input_mapping={}", host::backing(&bytes));
    let mut dense = new_file(&dir.join("allocated"))?;
    dense.write_all(&bytes)?;
    dense.sync_data()?;
    let mut sparse = new_file(&dir.join("holes"))?;
    sparse.set_len(size as u64)?;
    write_pages(Method::Zero, Backend::LIBRARY, &mut sparse, &bytes, page, Store::Native, 0)?;
    sparse.sync_data()?;
    let mut cases = vec![
        (Method::Dense, Backend::LIBRARY, Store::Native, 0),
        (Method::ExtentsAllocated, Backend::LIBRARY, Store::Native, 0),
        (Method::ExtentsSparse, Backend::LIBRARY, Store::Native, 0),
    ];
    for b in Backend::available() {
        for method in [
            Method::Zero,
            Method::Runs,
            Method::Diff,
            Method::Fill,
            Method::Hash,
            Method::StageCopy,
            Method::StageClear,
        ] {
            if b.name() == "asm-avx512-scan" && matches!(method, Method::Hash | Method::StageCopy | Method::StageClear)
            {
                continue;
            }
            let modes = match method {
                Method::StageCopy => Store::available(b),
                Method::StageClear => Store::for_clear(b),
                _ => &[Store::Native],
            };
            for &store in modes {
                let hints: &[usize] =
                    if method == Method::StageCopy && b.name().starts_with("asm") && store == Store::Native {
                        &[0, 128, 256, 512]
                    } else {
                        &[0]
                    };
                for &hint in hints {
                    cases.push((method, b, store, hint));
                }
            }
        }
    }
    for trial in 0..=rounds {
        if trial > 0 {
            cases.reverse();
        }
        for &(method, b, store, prefetch) in &cases {
            let dest = dir.join("output");
            let mut dst = new_file(&dest)?;
            let start = Instant::now();
            let result = match method {
                Method::ExtentsAllocated => extents(&mut dense, &mut dst, size),
                Method::ExtentsSparse => extents(&mut sparse, &mut dst, size),
                _ => write_pages(method, b, &mut dst, &bytes, page, store, prefetch),
            };
            if let Err(e) = result {
                if matches!(method, Method::ExtentsAllocated | Method::ExtentsSparse)
                    && matches!(e.raw_os_error(), Some(libc::EINVAL) | Some(libc::EOPNOTSUPP))
                {
                    eprintln!("{method:?} unavailable: {e}");
                    drop(dst);
                    std::fs::remove_file(dest)?;
                    continue;
                }
                return Err(e);
            }
            dst.set_len(size as u64)?;
            dst.sync_data()?;
            let elapsed = start.elapsed().as_nanos();
            let blocks = dst.metadata()?.blocks();
            verify(&mut dst, &bytes)?;
            if trial > 0 {
                println!("{trial},{page},{size},{method:?},{},{store:?},{prefetch},{elapsed},{blocks}", b.name());
            }
            drop(dst);
            std::fs::remove_file(dest)?;
        }
    }
    drop((dense, sparse));
    std::fs::remove_file(dir.join("allocated"))?;
    std::fs::remove_file(dir.join("holes"))?;
    std::fs::remove_dir(dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn full_partial_and_trailing_zero_runs_restore() -> io::Result<()> {
        let dir = std::env::temp_dir().join(format!("pryx-page-writer-{}", std::process::id()));
        std::fs::create_dir(&dir)?;
        let result = (|| -> io::Result<()> {
            let data = [0, 0, 0, 0, 7, 0, 9, 0, 0, 0, 0, 0, 13, 13, 13, 13, 0, 0, 0, 0];
            for backend in Backend::available() {
                for method in [
                    Method::Dense,
                    Method::Zero,
                    Method::Runs,
                    Method::Diff,
                    Method::Fill,
                    Method::Hash,
                    Method::StageCopy,
                    Method::StageClear,
                ] {
                    let path = dir.join("snapshot");
                    let mut file = new_file(&path)?;
                    write_pages(method, backend, &mut file, &data, 4, Store::Native, 0)?;
                    file.set_len(data.len() as u64)?;
                    verify(&mut file, &data)?;
                    let dest = dir.join("extents");
                    let mut copy = new_file(&dest)?;
                    match extents(&mut file, &mut copy, data.len()) {
                        Ok(()) => {
                            copy.set_len(data.len() as u64)?;
                            verify(&mut copy, &data)?;
                        }
                        Err(e) if matches!(e.raw_os_error(), Some(libc::EINVAL) | Some(libc::EOPNOTSUPP)) => {}
                        Err(e) => return Err(e),
                    }
                    drop((file, copy));
                    std::fs::remove_file(path)?;
                    std::fs::remove_file(dest)?;
                }
            }
            Ok(())
        })();
        std::fs::remove_dir_all(dir)?;
        result
    }
}
