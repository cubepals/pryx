# Bounded scan proof

Build the standalone sidecar with `cargo build --release --locked --features experimental --bin pryx-lab` from this directory. Its one operation is `pryx-lab self-check`. Without the Cargo feature this workspace contains only the scalar public entry point in the normal dependency graph. The installed scan reads whole 16-byte blocks using SSE2 or NEON after runtime detection, then checks the remaining bytes in Rust. Unsupported machines and Miri use the scalar predicate.

## Reproduce

```sh
cargo test --locked --features experimental
cargo clippy --locked --features experimental --all-targets -- -D warnings
rustup toolchain install nightly-2026-11-01 --profile minimal --component miri,rust-src
cargo +nightly-2026-11-01 miri test --locked --features experimental
cargo install --locked cargo-fuzz --version 0.13.2
cargo +nightly-2026-11-01 fuzz run scan -- -runs=10000 -max_len=65536
LLVM_MCA=llvm-mca-19 sh report.sh
```

The reports use LLVM 19.1.7 and OSACA 0.7.1. Both analyze the instructions between the source markers. Each loop iteration reads 16 bytes. LLVM's reciprocal block throughput is 0.8 cycles on Skylake Server and 1.0 on Neoverse N1. OSACA lacks two N1 instruction costs and withholds its final prediction. These static values exclude cache misses, Rust dispatch, setup, branches outside the loop and memory-system limits. Native measurements on matching cores are needed to compare them.

## Parts (`proof/`)

| Paths | Contents |
| --- | --- |
| `Cargo.toml`, `Cargo.lock`, `.gitignore` | Standalone workspace, pinned dependencies and local build outputs |
| `README.md`, `report.sh` | Test commands and model runner |
| `src/lib.rs`, `src/main.rs` | Scalar predicate, dispatch, differential input and sidecar entry point |
| `machine/Cargo.toml`, `machine/src/lib.rs` | Assembly crate and checked native call boundary |
| `machine/src/x86_64.S`, `machine/src/aarch64.S` | Complete-block scan loops |
| `tests/differential.rs`, `tests/guard_pages.rs` | Exhaustive one- and two-byte values, vector-tail cases, sparse and random inputs, inaccessible head and tail neighbors |
| `fuzz/Cargo.toml`, `fuzz/Cargo.lock`, `fuzz/fuzz_targets/scan.rs` | Separate coverage-guided differential target |
| `reports/x86_64-llvm-mca.txt`, `reports/x86_64-osaca.txt` | Skylake Server models |
| `reports/aarch64-llvm-mca.txt`, `reports/aarch64-osaca.txt` | Neoverse N1 models |
