#!/bin/sh
# Check the native assembly using libFuzzer's coverage-guided corpus on the pinned Rust compiler.
set -eu
cd "$(dirname "$0")"
target=$(rustc -vV | sed -n 's/^host: //p')
test -n "$target"
export RUSTFLAGS="-Cpasses=sancov-module -Cllvm-args=-sanitizer-coverage-level=3 -Cllvm-args=-sanitizer-coverage-inline-8bit-counters -Cllvm-args=-sanitizer-coverage-pc-table"
cargo build --locked --target "$target"
corpus=$1
mkdir -p "$corpus"
python3 - "$corpus" <<'PY'
from pathlib import Path
import sys
root = Path(sys.argv[1])
for n in (0, 1, 7, 15, 16, 17, 31, 32, 33, 63, 64, 65, 4096, 4097):
    (root / f"zero-{n}").write_bytes(bytes(n))
    (root / f"ramp-{n}").write_bytes(bytes(i % 251 for i in range(n)))
PY
exec "target/$target/debug/page" "$corpus" -runs=10000 -seed=42 -max_len=65536
