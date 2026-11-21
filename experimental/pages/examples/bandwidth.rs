// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Emit raw paired native samples. One invocation covers one page size and working set.
//! Buffer allocation, prefaulting, thread creation, and host reporting are outside each timer.
mod host;
use pryx_page_kernels::*;
use std::{hint::black_box, num::NonZeroUsize, sync::mpsc::sync_channel, time::Instant};

#[derive(Clone, Copy, Debug, PartialEq)]
enum Op {
    Read,
    ReadPair,
    Zero,
    Runs,
    Diff,
    Same,
    Hash,
    Copy,
    Clear,
}

struct Buffer {
    data: Vec<u8>,
    start: usize,
    len: usize,
}
impl Buffer {
    fn new(len: usize, offset: usize, byte: u8) -> Self {
        const ALIGN: usize = 2 << 20;
        let mut data = vec![0; len + ALIGN + 64];
        let start = data.as_ptr().align_offset(ALIGN) + offset;
        let active = &mut data[start..start + len];
        if let Ok(policy) = std::env::var("PRYX_LAB_THP") {
            let advice = match policy.as_str() {
                "on" => libc::MADV_HUGEPAGE,
                "off" => libc::MADV_NOHUGEPAGE,
                _ => panic!("THP policy"),
            };
            let granule = usize::try_from(unsafe { libc::sysconf(libc::_SC_PAGESIZE) }).unwrap();
            let skip = active.as_ptr().align_offset(granule);
            let len = active.len().saturating_sub(skip) / granule * granule;
            if len > 0 {
                assert_eq!(unsafe { libc::madvise(active.as_mut_ptr().add(skip).cast(), len, advice) }, 0);
            }
        }
        active.fill(byte);
        for b in active.iter_mut().step_by(4096) {
            unsafe { std::ptr::write_volatile(b, byte) }
        }
        Self { data, start, len }
    }
    fn bytes(&mut self) -> &mut [u8] {
        &mut self.data[self.start..self.start + self.len]
    }
}

#[derive(Clone, Copy)]
struct Case {
    backend: Backend,
    op: Op,
    store: Store,
    prefetch: usize,
}
fn execute(case: Case, page: usize, a: &[u8], b: &mut [u8]) -> u64 {
    let Case { backend, op, store, prefetch } = case;
    let mut acc = 0u64;
    if op == Op::Runs {
        runs(backend, black_box(a), NonZeroUsize::new(page).unwrap(), |range| {
            acc = acc.wrapping_add(black_box(range.len()) as u64)
        });
    } else {
        for (left, right) in a.chunks_exact(page).zip(b.chunks_exact_mut(page)) {
            let left = black_box(left);
            let right = black_box(right);
            let answer = match op {
                Op::Read => linear_read(left),
                Op::ReadPair => linear_read(left).wrapping_add(linear_read(right)),
                Op::Zero => zero(backend, left) as u64,
                Op::Same => same_value(backend, left) as u64,
                Op::Diff => first_diff(backend, left, right).unwrap_or(left.len()) as u64,
                Op::Hash => hash(backend, left),
                Op::Copy => {
                    copy(backend, store, right, left, prefetch);
                    black_box(right);
                    0
                }
                Op::Clear => {
                    clear(backend, store, right);
                    black_box(right);
                    0
                }
                Op::Runs => unreachable!(),
            };
            acc = acc.wrapping_add(answer);
        }
    }
    acc
}
#[inline(never)]
fn linear_read(bytes: &[u8]) -> u64 {
    bytes.as_chunks::<8>().0.iter().map(|b| u64::from_ne_bytes(*b)).fold(0, u64::wrapping_add)
}
fn probe(victim: &[u8]) -> u128 {
    let begin = Instant::now();
    black_box(linear_read(black_box(victim)));
    begin.elapsed().as_nanos()
}

fn cases() -> Vec<Case> {
    let mut result = Vec::new();
    for backend in Backend::available() {
        for op in [Op::Zero, Op::Runs, Op::Diff, Op::Same, Op::Hash, Op::Clear, Op::Copy, Op::Read, Op::ReadPair] {
            if matches!(op, Op::Read | Op::ReadPair) && backend != Backend::LIBRARY {
                continue;
            }
            for &store in if op == Op::Clear { Store::for_clear(backend) } else { Store::available(backend) } {
                if !matches!(op, Op::Copy | Op::Clear) && store != Store::Native {
                    continue;
                }
                // AVX-512 widens scans only. The hash and bulk bodies are the AVX2 candidates.
                if backend.name() == "asm-avx512-scan" && matches!(op, Op::Copy | Op::Clear | Op::Hash) {
                    continue;
                }
                let hints: &[usize] = if op == Op::Copy && backend.name().starts_with("asm") && store == Store::Native {
                    &[0, 128, 256, 512]
                } else {
                    &[0]
                };
                for &prefetch in hints {
                    result.push(Case { backend, op, store, prefetch })
                }
            }
        }
    }
    result
}
fn main() {
    let args: Vec<_> = std::env::args().skip(1).map(|s| s.parse::<usize>().expect("numeric argument")).collect();
    let [page, total, threads, offset, rounds]: [usize; 5] =
        args.try_into().expect("PAGE_BYTES TOTAL_WORKING_BYTES THREADS OFFSET ROUNDS");
    assert!(
        page.is_power_of_two()
            && threads > 0
            && total >= threads * page
            && total.is_multiple_of(threads * page)
            && offset < 64
            && rounds > 0
    );
    let per_thread = total / threads;
    // At least 32 MiB per sample amortizes synchronization on cache-resident buffers.
    let passes = (32usize << 20).div_ceil(total).max(1);
    let cases = cases();
    // Warm up once. Alternating order keeps input data and sample pairing stable.
    let count = cases.len();
    let schedule: Vec<_> = (0..=rounds)
        .flat_map(|trial| (0..count).map(move |i| (trial, if trial % 2 == 0 { i } else { count - 1 - i })))
        .collect();
    std::thread::scope(|scope| {
        let mut workers = Vec::with_capacity(threads);
        for worker in 0..threads {
            let (start, go) = sync_channel::<usize>(1);
            let (done, finished) = sync_channel(1);
            workers.push((start, finished));
            let cases = &cases;
            scope.spawn(move || {
                host::pin(worker);
                let mut a = Buffer::new(per_thread, offset, 0);
                let mut b = Buffer::new(per_thread, offset, 0);
                eprintln!(
                    "worker={worker} page={page} set={total} offset={offset} a_mapping={} b_mapping={}",
                    host::backing(a.bytes()),
                    host::backing(b.bytes())
                );
                let victim = vec![1; 256 << 10];
                done.send(None).unwrap();
                while let Ok(i) = go.recv() {
                    let store = matches!(cases[i].op, Op::Copy | Op::Clear);
                    let warm = if store {
                        probe(&victim);
                        probe(&victim)
                    } else {
                        0
                    };
                    let begin = Instant::now();
                    for _ in 0..passes {
                        black_box(execute(cases[i], page, a.bytes(), b.bytes()));
                    }
                    let end = Instant::now();
                    let after = if store { probe(&victim) } else { 0 };
                    done.send(Some((begin, end, warm, after))).unwrap();
                }
            });
        }
        for (_, finished) in &workers {
            finished.recv().unwrap();
        }
        for &(trial, i) in &schedule {
            for (start, _) in &workers {
                start.send(i).unwrap();
            }
            let samples: Vec<_> = workers.iter().map(|(_, finished)| finished.recv().unwrap().unwrap()).collect();
            let begin = samples.iter().map(|s| s.0).min().unwrap();
            let end = samples.iter().map(|s| s.1).max().unwrap();
            let ns = (end - begin).as_nanos();
            let warm = samples.iter().map(|s| s.2).sum::<u128>() / threads as u128;
            let after = samples.iter().map(|s| s.3).sum::<u128>() / threads as u128;
            let case = cases[i];
            if trial != 0 {
                println!(
                    "{trial},{page},{total},{threads},{offset},{passes},{:?},{},{:?},{},{ns},{warm},{after}",
                    case.op,
                    case.backend.name(),
                    case.store,
                    case.prefetch
                );
            }
        }
    });
}
