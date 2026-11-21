# Startup-specialized page scans

The standalone crate generates two fixed operations, a page-zero predicate and a staged copy. Runtime detection selects SSE2, AVX2, AVX-512F/BW or NEON, then groups one, two or four vectors according to the current CPU’s L1 data-cache line. For copies, x86 chooses REP MOVSB when FSRM is present and the size hint is at most 128 bytes, or when ERMS is present and the hint reaches a cache line. For a vector copy whose source and destination fit together in L1D, the emitter targets half a cache line per loop body. Larger spans target a full line. The artifact records the L1D geometry and choices for 16 bytes to 2 MiB. The Rust caller bounds each group and handles remainders. The assembler writes, relocates and seals one read/execute mapping. A host that declines this transition can still use its static or scalar scan. The explicit generated trial reports this refusal.

Use an unmodified checkout of AsmJit at `677a10bac2c24f7e068cf1f9e675bf51c7b534ea` as `ASMJIT_SOURCE`. CMake builds it only when the experimental feature is selected. No C++ runtime is added to the binary. From this directory run `cargo test --features experimental --locked`, `cargo run --features experimental --locked -- self-check`, or `cargo +nightly-2026-11-01 miri run --features experimental --locked -- self-check`. The latter exercises the scalar fallback.

The optional Measure dispatch sets `codegen_lab=true` and runs one architecture at a time. The paired writer pins itself to the first permitted CPU and recreates A1's 256 MiB resident image. Starting with seed 1, xorshift32 supplies the high byte for every byte of every tenth 4 KiB page. All other pages are zero. Every method receives one untimed-for-reporting warmup and 15 verified timed rounds in alternating reverse order. The timer covers each complete write through data sync, including hardware detection and assembly for each generated candidate. The collector checks the logical length and every restored byte after timing and records allocated bytes and CPU for each sample.

The static cohort includes A1's x86-64 winning Fill/AVX-512 scanner when available, its library comparator and AArch64's measured StageCopy/NEON with 512-byte prefetch. The transfer comparisons retain A1's library zero predicate for both generated and precompiled copy writers. A bulk dense write, dense staging, sparse fill, diff and zero writers, selected runs writers and the linked zero-scan plans compete on the same resident image. A1's ExtentsSparse requires an existing sparse disk file and belongs to its separate source-file study. The report compares each generated method with the fastest static complete writer on the current machine. It also retains finalized scan/copy encodings and their static byte comparisons. One architecture's result cannot establish graduation on both.

The x86 worker also times fixed short copies from 1 through 512 bytes and at 4 KiB. It compares direct libc memcpy, REP MOVSB when available, and each available vector plan on 64-byte-aligned buffers and distinct offsets (+1 source, +3 destination). Methods rotate in reverse order over 15 rounds after one warmup. Each timed batch performs 65,536 copies, with a repeated L1-sized working set. Output bytes and untouched sentinels are checked after every batch. At lengths smaller than a vector group its Rust tail handles the copy; the CSV records the native prefix length. These same-CPU comparisons describe the observed methods with ERMS/FSRM fixed. They do not estimate the effect of switching a CPU feature on or off.

## Parts (`codegen/`)

| Paths | Role |
| --- | --- |
| `Cargo.toml`, `Cargo.lock`, `.gitignore`, `README.md` | Feature-off workspace, pins, build output exclusions and commands |
| `src/main.rs`, `src/lib.rs`, `src/sleep.rs` | Self-check, shared writer entry, timed sparse writer |
| `machine/Cargo.toml`, `machine/CMakeLists.txt`, `machine/build.rs` | Standalone assembly crate and pinned, native build |
| `machine/src/lib.rs`, `machine/src/bulk.rs`, `machine/src/generator.cpp` | Checked CPU dispatch and fixed generated routines |
| `machine/src/x86_64.S`, `machine/src/aarch64.S` | Matched precompiled routines |
| `tests/guards.rs` | Fuzzed two-sided guard-page tests over every available native plan |
| `examples/short_copy.rs`, `short_copy_summary.py` | Fixed x86 short-copy characterization and paired summary |
| `comparison/Cargo.toml`, `comparison/Cargo.lock`, `comparison/src/main.rs` | Paired benchmark against each A1 backend |
| `report.py`, `summary.py`, `compare_images.py` | Disassemble/model static loops, summarize native writer samples and check emitted bytes against static variants |
| `reports/x86_64-llvm-mca.txt`, `reports/x86_64-osaca.txt`, `reports/aarch64-llvm-mca.txt`, `reports/aarch64-osaca.txt` | Static-loop predictions for Skylake Server and Neoverse N1; OSACA records each missing cost |
