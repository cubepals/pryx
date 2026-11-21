# Machine-code checks

The optional `validation_lab=true` dispatch in Measure builds the actual E0 scan from signed revision `2121b1ef2fdaf07d5b649264727de0fc10187df4`. It runs its Rust tests under Memcheck, emits generic and native Rust assembly, builds an executable containing the complete release-mode dispatch, and checks symbolic inputs against its return value. The same run samples E0's native L1 hot loop on its pinned CPU with a call-overhead control and a hardware cycle counter when available.

The symbolic input contains one unrestricted octet per byte, for lengths 0 through 64. The checker permits accesses inside the test slice and call stack and reads through the x86 ELF's RIP-relative call/jump slots. It checks all feasible paths return, all input values reach a checked return and no path can disagree with the scalar predicate. An invocation covers one specified pointer residue at a fixed base. Stack placement and CPU dispatch are those of the selected ELF. The proof applies only to the executable whose digest it emits.

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

## Parts (`validation/`)

| Paths | Role |
| --- | --- |
| `README.md`, `requirements.txt` | Scope, commands and pinned Python tool APIs |
| `adapter.rs`, `build.sh` | Build an exported function over the production predicate and emit an ELF |
| `prove.py` | Symbolically execute that ELF and check bounded access, coverage and return value |
| `memcheck.py`, `codegen.sh` | Run Cargo integration binaries under Memcheck and compare optimized Rust assembly |
| `hot_loop.c`, `model.py` | Native PMU and monotonic samples; LLVM scheduling-model comparison |
