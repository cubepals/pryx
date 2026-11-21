#!/usr/bin/env bash
# SPDX-License-Identifier: FSL-1.1-ALv2
# One private candidate admission reuses the staged ordinary-user VM toolchain.
# No credentials, VM memory, raw console output or test binaries leave the runner.
set -euo pipefail
umask 077
: "${RUNNER_TEMP:?stage the VM inputs first}"
: "${CARGO_TARGET_DIR:?use the same profile as the stock guest}"
lab=experimental/guest/candidate
root="$RUNNER_TEMP/lab-agent-candidate"
mkdir -m 0700 "$root"
export RUST_TARGET=x86_64-unknown-linux-musl
export MUSL_HEADERS="$RUNNER_TEMP/vm-cross/sysroot/usr/include"
export CC_x86_64_unknown_linux_musl=cc
export CFLAGS_x86_64_unknown_linux_musl="--sysroot=$RUNNER_TEMP/vm-cross/sysroot"
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER=cc
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_RUSTFLAGS="-C link-arg=--sysroot=$RUNNER_TEMP/vm-cross/sysroot"
production="$CARGO_TARGET_DIR/$RUST_TARGET/debug/pryx-agent"
service="$CARGO_TARGET_DIR/$RUST_TARGET/debug/lab-guest-service"
go="$RUNNER_TEMP/vm-build-tools/go/go/bin/go"
python3 experimental/guest/facts.py "$RUNNER_TEMP" "$production" "$root/host.json"
python3 experimental/guest/stage.py --candidate "$root/source"
cp "$root/source/lab-source.json" "$root/source.json"
cd "$root/source"
for variant in control activation first-request-proof; do
  bash "$lab/build.sh" "$root/$variant" "$variant"
done
cargo build --locked -p pryx-agent --target "$RUST_TARGET" --features lab-vsock-activation --bin lab-guest-service
"$production" bootstrap-probe > "$root/stock-roles.json"
"$service" bootstrap-probe > "$root/receiver-roles.json"
python3 -c 'import json,sys; a,b=[json.load(open(p)) for p in sys.argv[1:]]; assert a["vmInitVersion"] and a["vmVsock"] and a==b' \
  "$root/stock-roles.json" "$root/receiver-roles.json"
python3 "$lab/config.py" --go "$go" --stock "$RUNNER_TEMP/vm-config" --output "$root/config"
(
  cd templates/vm-tools
  CGO_ENABLED=0 GOTOOLCHAIN=local "$go" build -mod=readonly -trimpath -o "$root/first-requests" ./cmd/lab-gate-proof
)
for variant in trial proof runtime; do
  supervisor="$root/activation/lab-init"
  if [[ "$variant" == proof ]]; then supervisor="$root/first-request-proof/lab-init"; fi
  extra=()
  if [[ "$variant" != runtime ]]; then extra+=(--network-probe "$RUNNER_TEMP/vm-network-probe"); fi
  python3 templates/vm-tools/bootstrap.py \
    --agent "$production" --network-helper "$RUNNER_TEMP/vm-network-guest" --forwarder "$RUNNER_TEMP/vm-gvforwarder" \
    --filesystem-component "$RUNNER_TEMP/vm-filesystem-component" --root "$root/$variant-image" \
    --lab-supervisor "$supervisor" --lab-control "$root/control/lab-init" --lab-receiver "$service" "${extra[@]}"
done
export PRYX_LAB_TRIAL_BOOTSTRAP="$root/trial-image"
export PRYX_LAB_PROOF_BOOTSTRAP="$root/proof-image"
export PRYX_LAB_CONFIG_CONTROL="$root/config/vm-config-control"
export PRYX_LAB_CONFIG_ACTIVATION="$root/config/vm-config-activation"
export PRYX_LAB_GATE_PROBE="$root/first-requests"
python3 templates/vm-tools/agent_acceptance.py --lab-candidate \
  --inspection "$RUNNER_TEMP/vm-archive-inspection.json" --staged "$RUNNER_TEMP/vm-selected-files" \
  --configurator "$RUNNER_TEMP/vm-config" --client "$RUNNER_TEMP/vm-agent-client" \
  --host-launcher "$production" --bootstrap "$RUNNER_TEMP/vm-agent-bootstrap" \
  --filesystem-component "$RUNNER_TEMP/vm-filesystem-component" --proxy "$RUNNER_TEMP/vm-network-proxy" \
  --work "$root/boots"
python3 "$lab/evidence.py" "$root/boots/candidate-samples.json" > "$root/comparison.json"
python3 "$lab/bundle.py" "$RUNNER_TEMP/vm-runtime" "$root/runtime-image" "$PRYX_LAB_CONFIG_ACTIVATION" "$root/bundle"
export PRYX_VM_RUNTIME_DESCRIPTOR="$root/bundle/descriptor.json"
export PRYX_VM_RUNTIME_ARCHIVE="$root/bundle/runtime.tar.gz"
export PRYX_VM_HOST_LAUNCHER="$RUNNER_TEMP/vm-runtime/bin/vm-launcher"
export PRYX_VM_POLICY_PROBE="$RUNNER_TEMP/vm-network-probe"
export PRYX_VM_RUNTIME_DIAGNOSTICS="$root/private-diagnostics"
for test in actual_memory_sleep_keeps_a_running_process_and_falls_back_to_its_disk actual_forks_with_memory_wake_as_themselves; do
  name="runtime::vm::instance::memory_acceptance::$test"
  cargo test --locked --lib "$name" -- --list > "$root/lifecycle-list.txt"
  grep -Fx "$name: test" "$root/lifecycle-list.txt"
  cargo test --locked --lib "$name" -- --ignored --exact --test-threads 1 | tee "$root/lifecycle-last.log"
  grep -Fx "test $name ... ok" "$root/lifecycle-last.log"
  grep -Eq '^test result: ok[.] 1 passed; 0 failed;' "$root/lifecycle-last.log"
  echo "$test" >> "$root/lifecycle-passed.txt"
done
