#!/bin/sh
# SPDX-License-Identifier: FSL-1.1-ALv2
# Build the real process floor and fixed C workload against musl on a native Linux runner.
set -eu
destination=${1:?new output directory}
mkdir "$destination"
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
env CARGO_PROFILE_DEV_PANIC=abort "CC_$ccvar=musl-gcc" "CARGO_TARGET_${flag}_LINKER=musl-gcc" \
    cargo build --locked --no-default-features --features lab-observation -p pryx-agent \
        --example observation_floor --target "$target"
src=experimental/observation
unwind="$(rustc --print target-libdir --target "$target")/self-contained/libunwind.a"
musl-gcc -std=c11 -static -Wl,--gc-sections -O2 -Wall -Wextra -Werror -I"$destination/uapi" \
    "$src/workload.c" "$src/dispatch.c" "$src/trace.c" "$src/notification.c" "$src/getpid.S" \
    "${CARGO_TARGET_DIR:-target}/$target/debug/examples/libobservation_floor.a" "$unwind" -lpthread -ldl -lm \
    -o "$destination/observe"
