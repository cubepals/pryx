#!/bin/sh
# SPDX-License-Identifier: FSL-1.1-ALv2
# Build only the additional PID 1 program, keeping the serving agent untouched.
set -eu
target=${RUST_TARGET:?target triple for the staged musl toolchain}
cc=${CC:-cc}
headers=${MUSL_HEADERS:?verified musl headers for the target architecture}
out=${1:?output directory}
mkdir -p "$out"
define=
case "${2:-control}" in
  control) set -- --features lab-guest-split ;;
  activation) set -- --features lab-guest-supervisor-gate; define=-DLAB_ACTIVATION ;;
  first-request-proof) set -- --features lab-first-request-proof; define=-DLAB_ACTIVATION ;;
  *) exit 64 ;;
esac
cargo rustc "$@" --locked -p pryx-agent --target "$target" --example lab_guest_supervisor -- --print native-static-libs
archive=${CARGO_TARGET_DIR:-target}/"$target"/debug/examples/liblab_guest_supervisor.a
runtime=$(rustc --target "$target" --print target-libdir)/self-contained
test -f "$runtime/libunwind.a"
"$cc" ${define:+"$define"} -std=c11 -O2 -Wall -Wextra -Werror -ffunction-sections -fdata-sections \
  -nostdinc -isystem "$headers" -isystem "$("$cc" -print-file-name=include)" \
  -c experimental/guest/candidate/pid1.c -o "$out/pid1.o"
# Use the same self-contained target libc and unwinder as the stock Rust agent.
# The build host's C library and startup objects must never enter the guest ELF.
"$cc" -static -nostdlib -Wl,--gc-sections,-z,noexecstack -o "$out/lab-init" \
  "$runtime/crt1.o" "$runtime/crti.o" "$runtime/crtbegin.o" "$out/pid1.o" \
  -Wl,--start-group "$archive" "$runtime/libunwind.a" "$runtime/libc.a" -Wl,--end-group \
  "$runtime/crtend.o" "$runtime/crtn.o"
readelf -h -W "$out/lab-init" > "$out/elf-header.txt"
readelf -l -W "$out/lab-init" > "$out/elf-segments.txt"
if grep -Eq 'INTERP|DYNAMIC|GNU_STACK.*RWE' "$out/elf-segments.txt"; then
  exit 1
fi
