#!/bin/bash
# SPDX-License-Identifier: FSL-1.1-ALv2
# Run after the existing KVM workflow stages its production boot inputs.
set -euo pipefail
: "${RUNNER_TEMP:?private staging area}"
if (( $# > 1 )) || [[ ${1:-} != "" && ${1:-} != --stamp-only ]]; then exit 2; fi
src=experimental/guest
build="$RUNNER_TEMP/lab-inits"
sh "$src/build.sh" "$build"
target=x86_64-unknown-linux-musl
export CC_x86_64_unknown_linux_musl=cc
export CFLAGS_x86_64_unknown_linux_musl="--sysroot=$RUNNER_TEMP/vm-cross/sysroot"
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER=cc
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_RUSTFLAGS="-C link-arg=$CFLAGS_x86_64_unknown_linux_musl"
# Only the final link gets this entry stamp. Other crate artifacts are reused.
cargo rustc --locked -p pryx-agent --bin pryx-agent --target "$target" -- \
  -C "link-arg=$build/stamp.o" -C link-arg=-Wl,-e,pryx_lab_entry
cp "${CARGO_TARGET_DIR:-target}/$target/debug/pryx-agent" "$build/stamped-agent"
cargo build --locked -p pryx-agent --bin pryx-agent --target "$target"
python3 templates/vm-tools/bootstrap.py --agent "$build/stamped-agent" \
  --network-helper "$RUNNER_TEMP/vm-network-guest" --forwarder "$RUNNER_TEMP/vm-gvforwarder" \
  --filesystem-component "$RUNNER_TEMP/vm-filesystem-component" --root "$build/stamped-root"
if [[ ${1:-} == --stamp-only ]]; then exit 0; fi
python3 "$src/cohort.py" --staged "$RUNNER_TEMP" --build "$build" --agent "$build/stamped-root" \
  --work "$RUNNER_TEMP/lab-boots"
