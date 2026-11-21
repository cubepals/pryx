#!/bin/sh
# SPDX-License-Identifier: FSL-1.1-ALv2
# Link the actual E0 crate to a small entry point in a private scratch directory.
set -eu
if [ "$#" -ne 3 ]; then
    echo "usage: build.sh PROOF_CRATE TARGET OUT_ELF" >&2
    exit 2
fi
proof=$(cd "$1" && pwd)
target=$2
out=$3
here=$(cd "$(dirname "$0")" && pwd)
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT
mkdir -p "$scratch/app/src"
cp "$here/adapter.rs" "$scratch/app/src/main.rs"
ln -s "$proof" "$scratch/proof"
cp "$proof/Cargo.lock" "$scratch/app/Cargo.lock"
cat > "$scratch/app/Cargo.toml" <<'MANIFEST'
[package]
name = "pryx-proof-target"
version = "0.0.0"
edition = "2024"
publish = false
[dependencies]
pryx-lab-proof = { path = "../proof", features = ["experimental"] }
[workspace]
MANIFEST
cargo build --offline --release --manifest-path "$scratch/app/Cargo.toml" --target "$target" --target-dir "$scratch/target"
cp "$scratch/target/$target/release/pryx-proof-target" "$out"
