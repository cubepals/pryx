# Machine-code checks

The optional `validation_lab=true` dispatch in Measure builds the actual E0 scan from signed revision `2121b1ef2fdaf07d5b649264727de0fc10187df4`. It runs both E0 and the A1 crate at `07b4732183b684ab1775951314c3b5d3ae99ba74` under Memcheck, records the visible instruction families with and without instrumentation, emits generic and native Rust assembly for both crates, builds an executable containing the complete release-mode dispatch, and checks symbolic inputs against its return value. The same run samples E0 and A1 on a pinned CPU, using a matching empty call and a hardware cycle counter when available.

The symbolic input contains one unrestricted octet per byte, for lengths 0 through 64. The checker permits accesses inside the test slice and call stack and reads through the x86 ELF's RIP-relative call/jump slots. It checks all feasible paths return, all input values reach a checked return and no path can disagree with the scalar predicate. An invocation covers one specified pointer residue at a fixed base. Stack placement and CPU dispatch are those of the selected ELF. Nonargument registers start at zero. The result covers return values and tracked memory accesses; arbitrary caller register state and callee-saved register preservation are outside the proof. The proof applies only to the executable whose digest it emits.

`--stock-vex` selects stock angr. Its Arm engine lacks `Iop_ZeroHI120ofV128`. The default completes that single VEX operation using the low eight input bits and 120 zero bits according to `libvex_ir.h`. No guest instruction is replaced. The alternative SLEIGH engine stops at an unsupported vector CALLOTHER. The proof assumes the lifter's instruction semantics and the added bit-vector operation. It does not cover another compiler, A1 code, different pointer values, or lengths above the selected bound.

From this directory, with the E0 crate at `$PROOF` and the repository toolchain visible:

```sh
python3 -m venv "$CHECK_TOOLS"
"$CHECK_TOOLS/bin/pip" install -r requirements.txt
sh ./build.sh "$PROOF" "$(rustc -vV | awk '/^host:/{print $2}')" "$TMPDIR/scan.elf"
"$CHECK_TOOLS/bin/python" prove.py "$TMPDIR/scan.elf" --alignment 1
python3 memcheck.py "$PROOF/Cargo.toml" experimental
sh ./codegen.sh "$PROOF/Cargo.toml" experimental "$TMPDIR/scan-codegen"
```

For an Arm functional cross-check on x86, install the AArch64 musl standard library and run `CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=rust-lld CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_RUNNER=qemu-aarch64 cargo test --manifest-path "$PROOF/Cargo.toml" --target aarch64-unknown-linux-musl --features experimental --locked`. Valgrind exposes its own CPU model; the native test asserts assembly actually executed when detection reports support. The E0 kernel uses SSE2 or NEON. An AVX-512 executable needs a native check where Memcheck cannot decode those instructions.

The checker also receives three deliberately incorrect compiled controls. All pass at length zero. At length one the solver must reject an incorrect return and an extra byte read. A third control places the extra Arm load in the zero register, for which VEX omits the memory read. The checker compares instruction boundaries, memory direction and transferred byte width with unoptimized VEX in every reachable Arm block and refuses the proof if an effect is absent. Address and register semantics still depend on the lifter. This check covers the scan’s ordinary load/store instructions and refuses unfamiliar memory operations. These objects run only in the symbolic executor.

## Native page models

A1 runs the exact pinned equal-page comparison, zero/fill scan, four-lane fingerprint, copy and clear routines on aligned 4 KiB buffers. It also calls the ordinary cached Rust vector copy and clear. One buffer's working set is 4 KiB, and two take 8 KiB. The checker tests full-length scans, last-byte mismatches, every copied/cleared byte and all four reference hash lanes before collecting samples. The buffers are uniform during sampling. Every pair times 4,096 complete page calls and 4,096 empty calls with the same ABI, arguments and return checks. Fifteen pairs alternate order on one logical CPU. Each side gets fresh buffers and eight warm-up calls. Elapsed time and hardware counts include entry, tail, fence and return.

The A1 generator expands the actual assembly and reconstructs each aligned hot path. The analyzer checks that the instruction text is present verbatim in the pinned report, then reruns LLVM MCA and, where the CPU is supported, OSACA under the sampler’s CPU affinity. Nonzero copy hints are one block ahead. The final copy iteration takes the end-of-page guard. Fingerprint multiplication carries a dependence between blocks. LLVM’s block reciprocal throughput primarily describes resource demand. Its 100-iteration simulation is available in JSON for comparison. OSACA uses the greater of port pressure and loop-carried dependency cost. Transfer timings include storage and fence behavior that the loop models cannot predict. REP copy and clear and DC ZVA receive measured results where available and keep their instruction-model limitation. The one-time DC ZVA geometry probe has no loop throughput. Zero-run and equality compose the measured primitives in Rust.

Each model’s verdict compares the call-subtracted hardware median with its prediction, using the registered 20% relative-error threshold. A result requires 15 positive differences, user-mode CPU cycles enabled for the entire measured interval, and an available matching host model. The report records paired time in nanoseconds independently. The raw CSV, exact instruction paths, native executable, compiler/model output, median, nearest-rank p95 and unavailable reasons are in the `page-models` artifact directory. The pinned Skylake Server and Neoverse N1 estimates are recorded separately so a different host model remains identifiable.

## Parts (`validation/`)

| Paths | Role |
| --- | --- |
| `README.md`, `requirements.txt` | Scope, commands and pinned Python tool APIs |
| `adapter.rs`, `build.sh` | Build an exported function over the production predicate and emit an ELF |
| `controls.c`, `check_controls.py` | Check the solver against wrong-result and extra-read objects |
| `lifted_memory.py` | Refuse Arm proofs when a reachable memory instruction loses its effect in VEX |
| `prove.py` | Symbolically execute that ELF and check bounded access, coverage and return value |
| `memcheck.py`, `cpu_features.c`, `codegen.sh` | Record ISA coverage, run Cargo integration binaries under Memcheck and compare optimized Rust assembly |
| `cycles.h`, `hot_loop.c`, `model.py` | Shared pinning and PMU sampling and the E0 loop comparison |
| `page_loops.c`, `page_models.py` | A1 functional checks, model-path matching and host-model verdicts |
| `a1-vector.rs`, `a1-vector.sh` | Build and identify the emitted ordinary cached Rust transfers |
