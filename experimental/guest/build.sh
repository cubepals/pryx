#!/bin/sh
# SPDX-License-Identifier: FSL-1.1-ALv2
set -eu
src=$(dirname "$0")
out=${1:?new build directory}
mkdir "$out"
cc=${CC:-cc}
boot_clock=$(echo | "$cc" -dM -E -include linux/time.h - | awk '$2=="CLOCK_BOOTTIME" {print $3}')
test -n "$boot_clock"
"$cc" -c -DLAB_BOOT_CLOCK="$boot_clock" -o "$out/stamp.o" "$src/stamp.S"
for source in S c; do
    set --
    if [ "$source" = S ]; then
        set -- -Wl,-e,pryx_lab_entry "$out/stamp.o"
    fi
    "$cc" -Os -nostdlib -ffreestanding -fno-stack-protector -fno-pie -static \
        -Wl,--build-id=none "$@" "$src/init.$source" -o "$out/init-$source"
    readelf -hW "$out/init-$source"
    readelf -lW "$out/init-$source" > "$out/headers-$source"
    awk '$1 == "INTERP" || $1 == "DYNAMIC" {bad=1} END {exit bad}' "$out/headers-$source"
done
