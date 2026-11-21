// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2
//! Index a private source capture and byte-verify membership of pages in its independently
//! awakened fork. Only aggregate timings and counts can leave this process.

use super::{capture_replay::private_input, host};
use pryx_page_kernels::{Backend, hash};
use std::{
    collections::{BTreeSet, HashMap, hash_map::RandomState},
    fmt::Write as _,
    fs::{self, OpenOptions},
    io::{self, Write as _},
    os::unix::fs::OpenOptionsExt,
    path::PathBuf,
    time::Instant,
};

const PAGE: usize = 4096;
type Page = [u8; PAGE];
type Index<'a> = HashMap<u64, Vec<&'a Page>, RandomState>;

fn index_pages<'a>(source: &'a [Page], fingerprint: impl Fn(&Page) -> u64, seed: &RandomState) -> Index<'a> {
    let mut index = Index::with_capacity_and_hasher(source.len(), seed.clone());
    for page in source {
        let bucket = index.entry(fingerprint(page)).or_default();
        if !bucket.contains(&page) {
            bucket.push(page);
        }
    }
    index
}

fn membership(index: &Index<'_>, fork: &[Page], fingerprint: impl Fn(&Page) -> u64) -> Vec<bool> {
    fork.iter().map(|page| index.get(&fingerprint(page)).is_some_and(|bucket| bucket.contains(&page))).collect()
}

fn bytewise_oracle(source: &[Page], fork: &[Page]) -> Vec<bool> {
    // Lexicographic comparison of all 4096 bytes. No fingerprint or hash-table code is shared.
    let ordered: BTreeSet<&Page> = source.iter().collect();
    fork.iter().map(|page| ordered.contains(page)).collect()
}

fn report(source: &[Page], fork: &[Page]) -> io::Result<String> {
    if source == fork || source.is_empty() || source.len() != fork.len() {
        return Err(io::Error::other("expected a modified, same-sized fork capture"));
    }
    let oracle = bytewise_oracle(source, fork);
    let matched = oracle.iter().filter(|&&yes| yes).count();
    let nonzero = fork.iter().filter(|page| page.iter().any(|&byte| byte != 0)).count();
    let matched_nonzero = fork.iter().zip(&oracle).filter(|(page, yes)| **yes && page.iter().any(|&b| b != 0)).count();
    let changed_at_offset = source.iter().zip(fork).filter(|(a, b)| a != b).count();
    let mut methods = Backend::available();
    // This A1 backend is distinct only for scanning. Its hash runs the same native four-lane body.
    methods.retain(|b| b.name() != "asm-avx512-scan");
    let seed = RandomState::new();
    let mut csv = String::from(
        "pair,order,backend,page_bytes,source_pages,fork_pages,fork_nonzero_pages,changed_at_offset,matched_pages,matched_nonzero_pages,index_ns,probe_ns,total_ns\n",
    );
    for trial in 0..=15 {
        if trial > 0 {
            methods.reverse();
        }
        for (order, &backend) in methods.iter().enumerate() {
            let fingerprint = |page: &Page| hash(backend, page);
            let start = Instant::now();
            let index = index_pages(source, fingerprint, &seed);
            let built = Instant::now();
            let decisions = membership(&index, fork, fingerprint);
            let end = Instant::now();
            if decisions != oracle {
                return Err(io::Error::other("page membership differs from the independent bytewise oracle"));
            }
            if trial > 0 {
                let a = built.duration_since(start).as_nanos();
                let b = end.duration_since(built).as_nanos();
                writeln!(csv, "{trial},{order},{},{PAGE},{},{},{nonzero},{changed_at_offset},{matched},{matched_nonzero},{a},{b},{}",
                         backend.name(), source.len(), fork.len(), a + b).unwrap();
            }
        }
    }
    Ok(csv)
}

#[test]
fn captured_fork_lookup() -> io::Result<()> {
    host::pin(0);
    let path = |name| std::env::var_os(name).map(PathBuf::from).ok_or_else(|| io::Error::other("missing capture path"));
    let source = private_input(&path("PRYX_LAB_FORK_SOURCE")?)?;
    let fork = private_input(&path("PRYX_LAB_FORK_MEMORY")?)?;
    let target = path("PRYX_LAB_FORK_REPORT")?;
    let (source, src_tail) = source.as_chunks::<PAGE>();
    let (fork, fork_tail) = fork.as_chunks::<PAGE>();
    if !src_tail.is_empty() || !fork_tail.is_empty() {
        return Err(io::Error::other("partial final page in protected VM capture"));
    }
    let csv = report(source, fork)?;
    let staged = target.with_extension(format!("fork-{}.tmp", std::process::id()));
    let mut file = OpenOptions::new().write(true).create_new(true).mode(0o600).open(&staged)?;
    let outcome =
        file.write_all(csv.as_bytes()).and_then(|()| file.sync_data()).and_then(|()| fs::hard_link(&staged, target));
    drop(file);
    fs::remove_file(staged)?;
    outcome
}

#[test]
fn equal_nonzero_unmatched_and_colliding_pages() {
    let mut different_last_byte = [9; PAGE];
    different_last_byte[PAGE - 1] ^= 1;
    let source = [[0; PAGE], [9; PAGE], [9; PAGE], [3; PAGE]];
    let fork = [[9; PAGE], different_last_byte, [0; PAGE], [3; PAGE]];
    let expected = [true, false, true, true];
    assert_eq!(bytewise_oracle(&source, &fork), expected);
    let seed = RandomState::new();
    let collisions = index_pages(&source, |_| 0, &seed);
    assert_eq!(collisions.len(), 1);
    assert_eq!(collisions[&0].len(), 3);
    assert_eq!(membership(&collisions, &fork, |_| 0), expected);
    for b in Backend::available() {
        let fingerprint = |page: &Page| hash(b, page);
        assert_eq!(membership(&index_pages(&source, fingerprint, &seed), &fork, fingerprint), expected);
    }
}
