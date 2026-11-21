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
target_dir=$(cd "$target_dir" && pwd)
uapi="$destination/uapi"
unwind="$(rustc --print target-libdir --target "$target")/self-contained/libunwind.a"
build_fixture() {
    name=$1
    shift
    musl-gcc -std=c11 -static -Wl,--gc-sections -O2 -Wall -Wextra -Werror -I"$uapi" "$@" \
        "$src/workload.c" "$src/dispatch.c" "$src/trace.c" "$src/notification.c" "$src/getpid.S" \
        "$target_dir/$target/debug/examples/libobservation_floor.a" "$unwind" -lpthread -ldl -lm \
        -o "$destination/$name"
}
build_fixture observe
build_fixture observe-link -DPRYX_LINK_INSTRUMENTED -Wl,--wrap=fixed_getpid "$src/link-count.S"

# The prepared fixtures use fixed filenames in a private directory. Disable
# their build IDs so the arithmetic immediate is the only link output difference.
(
    output=$destination
    destination="$scratch/prepared"
    mkdir -m 700 "$destination"
    cd "$destination"
    prepared() {
        build_fixture "$1" -DPRYX_LINK_INSTRUMENTED -DPRYX_PREPARED_SLOT "-DPRYX_SLOT_INCREMENT=$2" \
            -Wl,--build-id=none,--wrap=fixed_getpid "$src/link-count.S"
    }
    prepared observe-prepared-off 0
    prepared linked-oracle 1
    objcopy -O binary --only-section=.pryx_observe_count observe-prepared-off inactive.slot
    objcopy -O binary --only-section=.pryx_observe_count linked-oracle oracle.slot
    python3 - <<'VERIFY'
import hashlib
import json
from pathlib import Path
import platform

arch = platform.machine()
# Exactly the existing wrapper's arithmetic instruction on the native target.
# Neither operand, the call destination, nor any other ELF byte may change.
encodings = {"x86_64": ("4183c200", "4183c201"), "aarch64": ("4a010011", "4a050011")}
before, after = (bytes.fromhex(word) for word in encodings[arch])
inactive = Path("observe-prepared-off").read_bytes()
oracle = Path("linked-oracle").read_bytes()
slot = Path("inactive.slot").read_bytes()
if slot.count(before) != 1 or inactive.count(slot) != 1:
    raise SystemExit("expected one counter arithmetic instruction in the prepared ELF's named section")
active_slot = slot.replace(before, after, 1)
if active_slot != Path("oracle.slot").read_bytes():
    raise SystemExit("the independent counter section differs outside the increment")
transformed = inactive.replace(slot, active_slot, 1)
if transformed != oracle:
    raise SystemExit("the prepared ELF differs from the independent active link")
with Path("observe-prepared-on").open("xb") as file:
    file.write(transformed)
Path("observe-prepared-on").chmod(0o700)
def digest(data):
    return hashlib.sha256(data).hexdigest()
proof = {"architecture": arch, "section": ".pryx_observe_count",
         "inactive_instruction_hex": before.hex(), "active_instruction_hex": after.hex(),
         "inactive_sha256": digest(inactive), "transformed_sha256": digest(transformed),
         "linked_oracle_sha256": digest(oracle)}
Path("prepared-verification.json").write_text(json.dumps(proof, indent=2) + "\n")
VERIFY
    mv observe-prepared-off observe-prepared-on prepared-verification.json "$output/"
)
