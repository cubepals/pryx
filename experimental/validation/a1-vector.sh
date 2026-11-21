#!/bin/sh
# SPDX-License-Identifier: FSL-1.1-ALv2
# Build from the pinned A1 source and retain the emitted assembly of the timed functions.
set -eu
if [ "$#" -ne 2 ]; then echo 'usage: a1-vector.sh A1_PAGES OUTPUT_DIR' >&2; exit 2; fi
pages=$(cd "$1" && pwd)
mkdir -p "$2"
out=$(cd "$2" && pwd)
here=$(cd "$(dirname "$0")" && pwd)
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT
mkdir -p "$scratch/pages"
# Keep explicit targets in Cargo.toml and the machine crate exactly as in A1.
for part in Cargo.toml Cargo.lock src machine examples; do cp -R "$pages/$part" "$scratch/pages/"; done
cat "$here/a1-vector.rs" >> "$scratch/pages/src/lib.rs"
cargo rustc --locked --offline --release --lib --features page-kernels \
    --manifest-path "$scratch/pages/Cargo.toml" --target-dir "$scratch/target" \
    --crate-type staticlib -- -C codegen-units=1 -C target-cpu=native --emit=asm,link
cp "$scratch/target/release/libpryx_page_kernels.a" "$out/vector.a"
set -- "$scratch"/target/release/deps/pryx_page_kernels-*.s
if [ "$#" -ne 1 ] || [ ! -s "$1" ]; then echo 'ambiguous A1 vector code listing' >&2; exit 1; fi
cp "$1" "$out/rust-vector-full.s"

# Preserve both wrapper bodies next to the archive; a libc call has no statically
# known instruction count or fixed transfer width for a native model.
python3 - "$out" <<'PYCODE'
import re
import sys
from pathlib import Path

out = Path(sys.argv[1])
listing = (out / "rust-vector-full.s").read_text()
for operation, library in (("copy", "memcpy"), ("clear", "memset")):
    symbol = "a1_vector_" + operation
    name = "rust-" + operation
    body = re.search(r"^" + symbol + r":\n(.*?)^\s*\.size\s+" + symbol + r",", listing, re.M | re.S)
    if not body:
        raise ValueError(f"native Rust assembly missing {symbol}")
    (out / (name + "-function.s")).write_text(symbol + ":\n" + body[1])
    # Every local back-edge is relevant: declining a model for unfamiliar code
    # is safer than modeling a residual tail as if it were the full page.
    labels = {line.rstrip(":"): i for i, line in enumerate(body[1].splitlines())
              if re.fullmatch(r"\.L\w+:", line)}
    backedges = []
    calls = []
    for i, line in enumerate(body[1].splitlines()):
        if line.lstrip().startswith("."):
            continue
        for label in re.findall(r"\.LBB[0-9]+_[0-9]+\b", line):
            if label in labels and labels[label] < i:
                backedges.append(line.strip())
        if re.search(r"\b" + library + r"\b", line):
            calls.append(line.strip())
    if len(calls) >= 2 and not backedges:
        reason = f"bulk span and tail delegate to {library}; no fixed Rust loop"
    else:
        reason = "no fixed bulk block identified for this compiler output"
    details = [reason, "library references:", *calls, "local back-edges:", *backedges]
    (out / (name + "-lowering.txt")).write_text("\n".join(details) + "\n")
PYCODE
