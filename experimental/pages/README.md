# Page experiments

The `page-kernels` feature enables this standalone crate. The production workspace has no
dependency on it. Scalar is always available. `Backend::available()` returns supported
alternatives for the measuring host. Miri excludes the assembly and Rust intrinsic candidates.

| Operation | Scalar reference | Maintained comparison | Rust intrinsics and assembly |
| --- | --- | --- | --- |
| Zero / contiguous runs | First nonzero byte; group adjacent zero pages | Compare to a zero page | AVX2, AVX-512BW or NEON compare, then the scalar tail |
| Equality / first difference | First differing byte or end of shorter input | Whole-slice equality, then locate mismatch | Compare paired vectors and locate the first unequal byte |
| Single value | Compare with first byte | Compare shifted slices | Broadcast first byte, reuse the zero scan |
| Page fingerprint | Four interleaved FNV-style 32-bit word lanes, a 64-bit fold over length, lanes, final bytes | BLAKE3, truncated to 64 bits | Parallel word xor and multiply. Compare bytes after a matching key |
| Copy / clear | Byte loop in ordinary Rust | Standard copy and fill | Cached Rust vectors. In assembly, x86 integer and vector streaming stores, REP MOVSB/STOSB, Arm LDNP/STNP, and DC ZVA for permitted 64-byte blocks |

Compile and test from this directory with `cargo test --locked --features page-kernels`.
The compiler can optimize plain Rust loops. Native codegen and host flags matter to interpreting
identical timings. Assembler-only loads stop before the slice boundary. Guard fixtures exercise
both a read and write adjacent to an unmapped-access boundary. Cross-architecture tests use
QEMU only for correctness. The architecture's native library dispatch is measured on a real CPU.

Dispatch **Measure** with `page_lab=memory` or `page_lab=sleep` on this branch. Each dispatch
runs the architectures in sequence and writes raw paired CSV, a summary TSV and machine
metadata. The memory run crosses one and two pinned workers, aligned and misaligned starts,
four logical page sizes, a cache-resident set and a set at least four times the reported LLC.
Reported bandwidth uses logical bytes divided by wall time; the paired control matches read,
two-read, copy or zero traffic. The 2 MiB cohort requests both disabled and enabled transparent
huge pages and records the actual mapping's backing. ERMS and FSRM are recorded by lscpu;
REP is measured directly on the same hardware. Copy prefetch distances are 0, 128, 256 and
512 bytes. Before each copy and clear, workers warm and read a disjoint 256 KiB victim buffer.
Reading it again after the stores measures cache pollution. Both victim probes are outside
the operation timer. Scheduling, cache bandwidth, and DRAM bandwidth belong to the machine in the artifact.

The sleep writer starts with 256 MiB of deterministic pages. One in ten 4 KiB pages holds
pseudorandom data. End each write with data sync and verify every restored byte outside the timer.
Allocated zeros and genuine holes are independent inputs to the extent-enumeration baseline.
The file writer has no VM dependency. Dense staging (clear) and sparse staging (copy) must each
be compared with the matching maintained path. Include fast dense writes and kernel extent
preservation when their input form applies. A file-level result alone cannot graduate a kernel.

| Parts | Contents |
| --- | --- |
| `Cargo.toml`, `Cargo.lock`, `.gitignore` | Isolated workspace, exact dependencies, generated-file exclusions |
| `src/lib.rs`, `src/dispatch.rs`, `src/reference.rs` | Slice operations, checked backend enumeration and scalar spec |
| `src/fingerprint.rs`, `src/bulk.rs` | Hash and copy/clear candidates |
| `src/x86_64.rs`, `src/aarch64.rs` | Rust intrinsic variants |
| `machine/Cargo.toml`, `machine/src/lib.rs` | Native ABI and Miri boundary |
| `machine/src/x86_64.S`, `machine/src/aarch64.S` | Hand-written implementations |
| `machine/reports/` | Static models, their generator and report index |
| `tests/differential.rs`, `tests/guard.rs` | Exhaustive values, alignments, tails, generated inputs and protected neighbors |
| `fuzz/Cargo.toml`, `fuzz/Cargo.lock`, `fuzz/targets/page.rs`, `fuzz/native.sh` | Isolated coverage-guided differential fuzz target |
| `examples/bandwidth.rs`, `examples/sleep_file.rs`, `examples/host/mod.rs` | Raw samples, verified writer and Linux CPU and mapping metadata |
| `measure.py` | Native matrix, provenance and nearest-rank percentiles |
