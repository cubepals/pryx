#!/bin/sh
# SPDX-License-Identifier: FSL-1.1-ALv2
# Build the real process floor and fixed C workload against musl on a native Linux runner.
set -eu
destination=${1:?new output directory}
mkdir "$destination"
destination=$(cd "$destination" && pwd)
root=$(cd "$(dirname "$0")/../.." && pwd)
src="$root/experimental/observation"
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT
case "$(uname -m)" in
    x86_64) target=x86_64-unknown-linux-musl ;;
    aarch64) target=aarch64-unknown-linux-musl ;;
    *) exit 2 ;;
esac
flag=$(echo "$target" | tr a-z- A-Z_)
ccvar=$(echo "$target" | tr - _)
mkdir "$destination/uapi"
for dir in linux asm-generic; do ln -s "/usr/include/$dir" "$destination/uapi/$dir"; done
ln -s "/usr/include/$(cc -print-multiarch)/asm" "$destination/uapi/asm"
if command -v rustup >/dev/null; then rustup target add "$target"; fi
# Cargo discovers only the baseline targets in the checkout. The opt-in runner
# declares its extra example in this private copy and resolves the original lock.
mkdir "$scratch/agent"
cp "$root/Cargo.toml" "$root/Cargo.lock" "$scratch/"
# Preserve the previous lab crate identity and feature graph inside this copy.
awk '{print} /^\[features\]$/ {print "lab-observation = [\"server\"]"}' \
    "$root/agent/Cargo.toml" > "$scratch/agent/Cargo.toml"
for path in src tests examples templates; do ln -s "$root/$path" "$scratch/$path"; done
for path in src tests examples; do ln -s "$root/agent/$path" "$scratch/agent/$path"; done
ln -s "$root/experimental" "$scratch/experimental"
cat >> "$scratch/agent/Cargo.toml" <<'MANIFEST'

[[example]]
name = "observation_floor"
path = "../experimental/observation/floor.rs"
crate-type = ["staticlib"]
required-features = ["lab-observation"]
MANIFEST
target_dir=${CARGO_TARGET_DIR:-"$scratch/target"}
env CARGO_PROFILE_DEV_PANIC=abort "CC_$ccvar=musl-gcc" "CARGO_TARGET_${flag}_LINKER=musl-gcc" \
    cargo build --locked --manifest-path "$scratch/Cargo.toml" --no-default-features --features lab-observation \
        -p pryx-agent --example observation_floor --target "$target" --target-dir "$target_dir"
cmp "$root/Cargo.lock" "$scratch/Cargo.lock"
unwind="$(rustc --print target-libdir --target "$target")/self-contained/libunwind.a"
build_fixture() {
    name=$1
    shift
    musl-gcc -std=c11 -static -Wl,--gc-sections -O2 -Wall -Wextra -Werror -I"$destination/uapi" "$@" \
        "$src/workload.c" "$src/dispatch.c" "$src/trace.c" "$src/notification.c" "$src/getpid.S" \
        "$target_dir/$target/debug/examples/libobservation_floor.a" "$unwind" -lpthread -ldl -lm \
        -o "$destination/$name"
}
build_fixture observe
build_fixture observe-link -DPRYX_LINK_INSTRUMENTED -Wl,--wrap=fixed_getpid "$src/link-count.S"
