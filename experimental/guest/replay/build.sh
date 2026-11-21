#!/bin/bash
# SPDX-License-Identifier: FSL-1.1-ALv2
# Build a supplemental child of the pinned writer module so it can use its unmodified
# private write/verify routines. Input is a distinct checkout. Only the private build
# directory gets the appended module declaration. No guest memory is present at build time.
set -euo pipefail
umask 077
test "$#" -eq 2
adapter=$(cd "$(dirname "$0")" && pwd)
checkout=$(cd "$1" && pwd)
pin=07b4732183b684ab1775951314c3b5d3ae99ba74
test "$(git -C "$checkout" rev-parse HEAD)" = "$pin"
mkdir -m 700 -- "$2"
target=$(cd "$2" && pwd)
git -C "$checkout" archive "$pin" experimental/pages | tar -x -C "$target"
workspace=$target/experimental/pages
cp "$adapter/capture.rs" "$workspace/examples/capture.rs"
printf '\n#[path = "capture.rs"]\nmod capture_replay;\n' >> "$workspace/examples/sleep_file.rs"
export CARGO_TARGET_DIR="$target/compiled"
cargo test --locked --release --no-run --features page-kernels \
    --manifest-path "$workspace/Cargo.toml" --example sleep_file --message-format=json > "$target/artifacts.json"
python3 - "$target" <<'PY'
import json
from pathlib import Path
import shutil
import sys
root = Path(sys.argv[1])
messages = (json.loads(line) for line in (root / "artifacts.json").read_text().splitlines())
binaries = [m["executable"] for m in messages if m.get("reason") == "compiler-artifact"
            and m["target"]["name"] == "sleep_file" and m.get("executable") and m["profile"]["test"]]
binary, = binaries
shutil.copy2(binary, root / "replay-tests")
PY
"$target/replay-tests" --list --exact capture_replay::captured_image | grep -Fx 'capture_replay::captured_image: test' > /dev/null
"$target/replay-tests" --exact capture_replay::protected_small_fixture
install -m 700 "$adapter/run.sh" "$target/replay"
