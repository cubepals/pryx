#!/bin/bash
# SPDX-License-Identifier: FSL-1.1-ALv2
set -euo pipefail
: "${RUNNER_TEMP:?private staging area}"
# Compile and boot only after the mandatory samples have been uploaded.
src=experimental/guest
build="$RUNNER_TEMP/lab-inits"
kernels="$RUNNER_TEMP/lab-kernels"
mkdir -m 700 "$kernels"
# Leave two minutes for the final upload and reserve time for the two 15-boot cohorts.
# The original runtime inventory and mandatory guest evidence already uploaded.
remaining=$(( ${PRYX_LAB_KERNEL_END:?job deadline} - $(cut -d. -f1 /proc/uptime) ))
if (( remaining < 17 * 60 )); then
  printf 'kernel pair unattempted: %s seconds remain before artifact reserve, 1020 needed\n' "$remaining" > "$kernels/build-status.txt"
  exit 0
fi
set +e
started=$(cut -d. -f1 /proc/uptime)
timeout --kill-after=5s 895s bash "$src/kernel_toolchain.sh" >"$kernels/build.log" 2>&1
status=$?
set -e
elapsed=$(( $(cut -d. -f1 /proc/uptime) - started ))
rm -rf -- "$kernels/build"
printf 'kernel pair build exit %s; elapsed %ss including setup; hard allowance 900s\n' "$status" "$elapsed" >"$kernels/build-status.txt"
if (( status == 0 )); then
  remaining=$(( PRYX_LAB_KERNEL_END - $(cut -d. -f1 /proc/uptime) ))
  if (( remaining <= 0 )); then
    printf 'kernel pair boot unattempted: artifact reserve reached\n' >> "$kernels/build-status.txt"
    exit 0
  fi
  boot_status=0
  timeout --kill-after=5s "${remaining}s" python3 "$src/cohort.py" --staged "$RUNNER_TEMP" --build "$build" --agent "$build/stamped-root" \
    --kernel-pair "$RUNNER_TEMP/lab-kernels" --work "$RUNNER_TEMP/lab-kernel-pair" || boot_status=$?
  printf 'matched 30-boot cohort exit %s\n' "$boot_status" >> "$kernels/build-status.txt"
fi
