# Static page-kernel models

The reports contain the complete assembly path and both tools' output for every handwritten assembly entry point. LLVM MC expands the source macros first. LLVM MCA 19.1.7 models Skylake Server and Neoverse N1. OSACA 0.7.1 uses SKX and N1 with flag dependencies enabled. The model includes each instruction executed by a successful full-block scan or a fully aligned transfer iteration, including its conditional branches. Copies have a zero-hint path and a separate path for a nonzero, in-bounds prefetch. Mismatch exits, scalar tails, alignment prologues, setup and return fences are outside these loop totals.

LLVM's block reciprocal throughput estimates resource demand in cycles per iteration. Loop-carried dependencies can take longer, especially for the fingerprint multiplier. Memory bandwidth, cache residency, page faults, branch mispredictions, and useful hardware prefetching require measurements. The REP string engine has a runtime byte count; the modeled instruction costs cannot be divided into per-byte throughput. DC ZVA also depends on cache state and hardware behavior. OSACA marks missing entries with X and withholds its final prediction. The warning count below is the number of unmodeled instructions in each path.

From the repository root, run `python3 experimental/pages/machine/reports/generate.py`. Set `LLVM_MC`, `LLVM_MCA` and `OSACA` if their pinned versions are outside the command search path. The generator removes OSACA's wall-clock stamp. Its path walker refuses unrecognized assembly entry points. Compiler-generated Rust and maintained-library baselines are timed by the measurement harness.

## Parts (`reports/`)

`README.md` lists the model inputs, assumptions and every report. `generate.py` reads the two architecture assembly sources and regenerates the files below.

| Report | Path order | LLVM cycles/block | OSACA missing |
| --- | --- | ---: | ---: |
| [aarch64-clear_nt.txt](aarch64-clear_nt.txt) | aligned whole block | 2.0 | 5 |
| [aarch64-clear_zva.txt](aarch64-clear_zva.txt) | aligned 64-byte interior | 1.0 | 2 |
| [aarch64-copy_nt.txt](aarch64-copy_nt.txt) | no prefetch / with prefetch | 3.0 / 4.0 | 7 / 9 |
| [aarch64-diff_neon.txt](aarch64-diff_neon.txt) | whole equal block | 3.0 | 8 |
| [aarch64-hash_lanes.txt](aarch64-hash_lanes.txt) | full 16-byte group | 2.0 | 3 |
| [aarch64-value_neon.txt](aarch64-value_neon.txt) | whole equal block | 3.0 | 8 |
| [aarch64-zva64.txt](aarch64-zva64.txt) | one-time DC ZVA capability probe | probe only | 4 |
| [x86_64-clear_gpr.txt](x86_64-clear_gpr.txt) | aligned whole block | 2.0 | 0 |
| [x86_64-clear_nt.txt](x86_64-clear_nt.txt) | aligned whole block | 2.0 | 0 |
| [x86_64-clear_rep.txt](x86_64-clear_rep.txt) | count in RCX; internal string loop is microcoded | variable count | 1 |
| [x86_64-copy_gpr.txt](x86_64-copy_gpr.txt) | no prefetch / with prefetch | 2.5 / 3.0 | 0 / 1 |
| [x86_64-copy_nt.txt](x86_64-copy_nt.txt) | no prefetch / with prefetch | 2.5 / 3.0 | 0 / 1 |
| [x86_64-copy_rep.txt](x86_64-copy_rep.txt) | count in RCX; internal string loop is microcoded | variable count | 1 |
| [x86_64-diff_avx2.txt](x86_64-diff_avx2.txt) | whole equal block | 2.0 | 0 |
| [x86_64-diff_avx512.txt](x86_64-diff_avx512.txt) | whole equal block | 2.0 | 2 |
| [x86_64-hash_lanes.txt](x86_64-hash_lanes.txt) | full 16-byte group | 1.2 | 0 |
| [x86_64-value_avx2.txt](x86_64-value_avx2.txt) | whole equal block | 1.8 | 0 |
| [x86_64-value_avx512.txt](x86_64-value_avx512.txt) | whole equal block | 1.7 | 1 |
