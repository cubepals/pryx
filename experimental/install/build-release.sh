#!/bin/sh
# SPDX-License-Identifier: FSL-1.1-ALv2
# Build for this native release runner. Verify static linkage and exercise the binary before signing.
set -eu
arch=$1
destination=$2
case "$arch/$(uname -s)/$(uname -m)" in
  x64/Linux/x86_64) target=x86_64-unknown-linux-musl ;;
  arm64/Linux/aarch64) target=aarch64-unknown-linux-musl ;;
  *) echo "lab release architecture does not match this native runner" >&2; exit 1 ;;
esac
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT
CARGO_TARGET_DIR="$scratch/build" RUSTFLAGS='-C linker=rust-lld' cargo build \
  --release --locked --target "$target" --manifest-path experimental/proof/Cargo.toml --features experimental
binary="$scratch/build/$target/release/pryx-lab"
LC_ALL=C readelf --program-headers --dynamic --wide "$binary" > "$scratch/linkage"
if grep -Eq 'INTERP|\(NEEDED\)' "$scratch/linkage"; then
  echo "lab release executable requires a host dynamic linker or library" >&2
  exit 1
fi
[ "$("$binary" self-check)" = 'self-check passed' ]
mkdir -p "$destination"
python3 -B experimental/install/package.py "$binary" "$destination/pryx-lab-linux-$arch.pyz" --arch "$arch"
