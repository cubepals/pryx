#!/bin/bash
# SPDX-License-Identifier: FSL-1.1-ALv2
# Build with symbols, then profile the stock release-mode storage benchmark in an ordinary account.
set -euo pipefail
python3 -B experimental/storage-profile/record_test.py
out="$RUNNER_TEMP/storage-profile"
mkdir -p "$out"
lscpu --json > "$out/cpu.json"
uname -a > "$out/kernel"
findmnt -T "$RUNNER_TEMP" > "$out/filesystem"
cargo test --release --locked --test storage_bench --no-run --message-format=json > "$RUNNER_TEMP/storage-build.jsonl"
binary=$(jq -sr '[.[] | select(.reason == "compiler-artifact" and .target.name == "storage_bench" and .executable != null) | .executable] | unique | if length==1 then .[0] else error("missing storage benchmark") end' "$RUNNER_TEMP/storage-build.jsonl")
readelf --sections --wide "$binary" > "$RUNNER_TEMP/storage-sections.txt"
grep -q "\.symtab " "$RUNNER_TEMP/storage-sections.txt"
perf=$(command -v perf)
if ! "$perf" version; then
  for candidate in /usr/lib/linux-tools/*/perf /usr/lib/linux-tools-*/perf; do
    if [ -x "$candidate" ] && "$candidate" version; then perf=$candidate; break; fi
  done
fi
"$perf" version > "$out/perf-version"
python3 experimental/storage-profile/record.py "$perf" "$binary" "$out"
