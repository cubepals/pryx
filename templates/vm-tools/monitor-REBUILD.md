# Rebuilding and relinking the separate VM monitor

The runtime's Firecracker program is Apache-2.0, built from the complete pinned original
source tree with GNU libc (LGPL-2.1-or-later). GCC's static runtime uses its Runtime Library
Exception. Primary notices accompany the binary; the separate corresponding-source archive
contains the original Firecracker tree, all resolved normal/build Rust dependency sources,
complete original libc source, exact Buildroot recipes/configuration, and these build tools.
The compiler SDK itself is a downloaded build input, not part of the distributed runtime.

You may modify or relink this separate monitor with a modified GNU libc. Reverse engineering
for debugging modifications to that library is permitted. No pryx key or service is required.

With Python 3.11+, Git, binutils, a host C compiler, system libseccomp and rustup available:

1. Enter the `monitor/sources` directory of the full corresponding-source distribution (or
   `sources` in the standalone monitor component). Keep its `inputs.json` and original
   Firecracker archive together. `build-tools/monitor-inputs.json` pins the compiler and every
   additional source/notice download. All downloads require matching byte counts and SHA-256.
2. Run `python3 build-tools/monitor_prepare.py --root /absolute/private/sdk --cache /absolute/private/cache`.
3. Run `python3 build-tools/monitor_build.py --sources "$PWD" --sdk /absolute/private/sdk --root /absolute/private/build --target-dir /absolute/private/target`.

The build uses the Rust version declared by the original monitor's `rust-toolchain.toml`.
Cargo.lock fixes its dependencies. Original registry archives and the full micro-http Git
source are retained under `monitor-rust`; Cargo may fetch the same immutable inputs into its
normal cache. Build-only libseccomp stays in host build tools. The monitor is a static PIE;
its native signal/futex/syscall values are imported from its actual source/libc. The policy adapter
retains the published trap-default policy and changes only the equivalent libc thread/event-wait/terminal ABI and file metadata (`statx` beside the published `stat`, which snapshot creation reads through `std::fs::File::metadata`), including the read of its own PID required by GNU thread signalling. The
build independently recompiles this policy, requires those exact BPF bytes in the ELF, and executes real thread kicks/joins under all three actual thread profiles and the maintained terminal restore call under the VMM profile without KVM.

To change libc, follow the publisher's retained `compiler-recipe/README` and `defconfig`
with the full `buildroot-toolchains` source archive. The original libc source is alongside it.
Use Buildroot's `GLIBC_OVERRIDE_SRCDIR` in `local.mk` to name your modified libc source before
the fresh build, then build the SDK as the publisher describes. Extract your replacement SDK
under a private directory, preserving the original SDK directory name, and run its relocation
script. Pass that private directory to `monitor_build.py --sdk`; the application is rebuilt
and relinked against your modified libc. Keep the self-process launcher confinement and run
the real KVM lifecycle/shutdown suite before using a changed monitor with pryx.
