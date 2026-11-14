#!/usr/bin/env bash
# SPDX-License-Identifier: FSL-1.1-ALv2
# Build phases for the separately inventoried VM distributions. This recipe does not
# publish, install host packages, change host policy or start a guest. The caller owns
# its private RUNNER_TEMP directory, working directory and build environment.
set -euo pipefail
: "${RUNNER_TEMP:?Provide a private VM build directory}"
case "${1:?Name one VM build phase}" in
  inventory)
    python3 templates/vm-tools/inventory.py \
      --archive-cache "$RUNNER_TEMP/vm-archives" \
      --output "$RUNNER_TEMP/vm-candidate-inventory.json" | tee "$RUNNER_TEMP/vm-input-inventory.log"
    ;;
  tools)
    python3 templates/vm-tools/prepare.py --root "$RUNNER_TEMP/vm-build-tools" | tee "$RUNNER_TEMP/vm-tool-inputs.log"
    dpkg-query -W e2fsprogs
    ;;
  inspect)
    "$RUNNER_TEMP/vm-build-tools/zstd/usr/bin/zstd" --version
    python3 templates/vm-tools/archive_inspection.py \
      --zstd "$RUNNER_TEMP/vm-build-tools/zstd/usr/bin/zstd" \
      --inventory "$RUNNER_TEMP/vm-candidate-inventory.json" \
      --archive-cache "$RUNNER_TEMP/vm-archives" \
      --staged "$RUNNER_TEMP/vm-selected-files" \
      --output "$RUNNER_TEMP/vm-archive-inspection.json" | tee "$RUNNER_TEMP/vm-archive-inspection.log"
    ;;
  configurator)
    pryx_go="$RUNNER_TEMP/vm-build-tools/go/go/bin/go"
    "$pryx_go" mod verify
    python3 notices.py --go "$pryx_go" --output "$RUNNER_TEMP/vm-spike-notices" \
      --packages ./cmd/config ./cmd/guest ./cmd/agent
    "$pryx_go" build -mod=readonly -trimpath -o "$RUNNER_TEMP/vm-config" ./cmd/config
    "$pryx_go" build -mod=readonly -trimpath -o "$RUNNER_TEMP/vm-guest" ./cmd/guest
    "$pryx_go" build -mod=readonly -trimpath -o "$RUNNER_TEMP/vm-agent-client" ./cmd/agent
    "$pryx_go" mod verify
    ;;
  filesystems)
    python3 templates/vm-tools/filesystem.py \
      --cache "$RUNNER_TEMP/vm-filesystem-archives" \
      --root "$RUNNER_TEMP/vm-filesystem-component" | tee "$RUNNER_TEMP/vm-filesystem-component.log"
    python3 templates/vm-tools/filesystem_check.py \
      --component "$RUNNER_TEMP/vm-filesystem-component" \
      --output "$RUNNER_TEMP/vm-filesystem-check.json"
    ;;
  network-inputs)
    python3 templates/vm-tools/network_prepare.py --root "$RUNNER_TEMP/vm-network-inputs" | tee "$RUNNER_TEMP/vm-network-inputs.log"
    ;;
  network)
    pryx_go="$RUNNER_TEMP/vm-build-tools/go/go/bin/go"
    "$pryx_go" mod verify
    python3 ../notices.py --go "$pryx_go" --module-root "$PWD" \
      --catalog "$PWD/module-notices.json" --output "$RUNNER_TEMP/vm-network-notices" \
      --packages ./cmd/proxy ./cmd/probe ./cmd/guest github.com/containers/gvisor-tap-vsock/cmd/vm
    "$pryx_go" build -mod=readonly -trimpath -o "$RUNNER_TEMP/vm-network-proxy" ./cmd/proxy
    "$pryx_go" build -mod=readonly -trimpath -o "$RUNNER_TEMP/vm-network-probe" ./cmd/probe
    "$pryx_go" build -mod=readonly -trimpath -o "$RUNNER_TEMP/vm-network-guest" ./cmd/guest
    "$pryx_go" build -mod=readonly -trimpath -o "$RUNNER_TEMP/vm-gvforwarder" github.com/containers/gvisor-tap-vsock/cmd/vm
    "$pryx_go" mod verify
    ;;
  bootstrap)
    python3 templates/vm-tools/cross.py --root "$RUNNER_TEMP/vm-cross" \
      --cache "$RUNNER_TEMP/vm-filesystem-archives" | tee "$RUNNER_TEMP/vm-cross.log"
    rustup target add x86_64-unknown-linux-musl
    export CC_x86_64_unknown_linux_musl=cc
    export CFLAGS_x86_64_unknown_linux_musl="--sysroot=$RUNNER_TEMP/vm-cross/sysroot"
    export CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER=cc
    export CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_RUSTFLAGS="-C link-arg=--sysroot=$RUNNER_TEMP/vm-cross/sysroot"
    cargo build --locked -p pryx-agent --target x86_64-unknown-linux-musl
    cargo build --locked --manifest-path vm-image/Cargo.toml --target x86_64-unknown-linux-musl
    python3 templates/vm-tools/bootstrap.py \
      --agent target/x86_64-unknown-linux-musl/debug/pryx-agent \
      --network-helper "$RUNNER_TEMP/vm-network-guest" \
      --forwarder "$RUNNER_TEMP/vm-gvforwarder" \
      --network-probe "$RUNNER_TEMP/vm-network-probe" \
      --filesystem-component "$RUNNER_TEMP/vm-filesystem-component" \
      --root "$RUNNER_TEMP/vm-agent-bootstrap" | tee "$RUNNER_TEMP/vm-agent-bootstrap.log"
    ;;
  sources)
    python3 templates/vm-tools/sources.py --cache "$RUNNER_TEMP/vm-source-cache" \
      --output "$RUNNER_TEMP/vm-corresponding-sources" | tee "$RUNNER_TEMP/vm-corresponding-sources.log"
    ;;
  bundle)
    python3 templates/vm-tools/bootstrap.py \
      --agent target/x86_64-unknown-linux-musl/debug/pryx-agent \
      --network-helper "$RUNNER_TEMP/vm-network-guest" --forwarder "$RUNNER_TEMP/vm-gvforwarder" \
      --filesystem-component "$RUNNER_TEMP/vm-filesystem-component" --root "$RUNNER_TEMP/vm-runtime-bootstrap"
    python3 templates/vm-tools/bundle.py \
      --inspection "$RUNNER_TEMP/vm-archive-inspection.json" --staged "$RUNNER_TEMP/vm-selected-files" \
      --sources "$RUNNER_TEMP/vm-corresponding-sources" --source-output "$RUNNER_TEMP/vm-bundle-sources" \
      --filesystem-component "$RUNNER_TEMP/vm-filesystem-component" \
      --sdk-notices "$RUNNER_TEMP/vm-spike-notices" --network-notices "$RUNNER_TEMP/vm-network-notices" \
      --bootstrap "$RUNNER_TEMP/vm-runtime-bootstrap" \
      --launcher target/x86_64-unknown-linux-musl/debug/pryx-agent \
      --image-helper target/x86_64-unknown-linux-musl/debug/pryx-vm-image \
      --configurator "$RUNNER_TEMP/vm-config" --transport "$RUNNER_TEMP/vm-agent-client" \
      --network-proxy "$RUNNER_TEMP/vm-network-proxy" --output "$RUNNER_TEMP/vm-runtime" \
      --archive "$RUNNER_TEMP/pryx-vm-linux-x64.tar.gz" --source-archive "$RUNNER_TEMP/pryx-vm-sources.tar.gz" \
      --descriptor "$RUNNER_TEMP/vm-runtime.json" --source-descriptor "$RUNNER_TEMP/vm-sources.json" \
      --url-base https://github.com/cubepals/pryx/releases/download/preview | tee "$RUNNER_TEMP/vm-bundle.log"
    ;;
  *) echo "Unknown VM build phase" >&2; exit 1 ;;
esac
