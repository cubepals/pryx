#!/bin/sh
# SPDX-License-Identifier: FSL-1.1-ALv2
# Keep the Rust optimizer's generic and native assembly for this runner/compiler.
set -eu
if [ "$#" -ne 3 ]; then echo 'usage: codegen.sh MANIFEST FEATURE OUTPUT_DIR' >&2; exit 2; fi
manifest=$1
feature=$2
out=$3
mkdir -p "$out"
out=$(cd "$out" && pwd)
rustc -vV > "$out/rustc.txt"
for cpu in generic native; do
    scratch=$(mktemp -d)
    trap 'rm -rf "$scratch"' EXIT
    cargo rustc --manifest-path "$manifest" --lib --release --locked --features "$feature" \
        --target-dir "$scratch" -- --emit=asm -C "target-cpu=$cpu"
    set -- "$scratch"/release/deps/*.s
    if [ "$#" -ne 1 ] || [ ! -f "$1" ]; then echo 'ambiguous Rust assembly output' >&2; exit 1; fi
    cp "$1" "$out/$cpu.s"
    rm -rf "$scratch"
done
status=0
diff -u "$out/generic.s" "$out/native.s" > "$out/native.diff" || status=$?
[ "$status" -le 1 ]
