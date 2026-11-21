#!/bin/sh
# SPDX-License-Identifier: FSL-1.1-ALv2
# Regenerate the analytical predictions for the marked hot loops.
set -eu
cd "$(dirname "$0")"
mca=${LLVM_MCA:-llvm-mca}
osaca=${OSACA:-osaca}
destination=${1:-reports}
mkdir -p -- "$destination"
build_reports() {
    arch=$1
    cpu=$2
    model=$3
    input=machine/src/$arch.S
    {
        "$mca" --version | head -n 1
        echo "input $input, -mtriple=$arch -mcpu=$cpu -iterations=100"
        awk '/OSACA-BEGIN/ { body = 1; print ".Lblocks:"; next } /OSACA-END/ { exit } body' "$input" |
            "$mca" "-mtriple=$arch" "-mcpu=$cpu" -iterations=100
    } > "$destination/$arch-llvm-mca.txt"
    {
        "$osaca" -V
        echo "input $input, --arch $model"
        awk '/OSACA-BEGIN/ { body = 1; next } /OSACA-END/ { exit } body' "$input" |
            "$osaca" --arch "$model" --consider-flag-deps /dev/stdin
    } > "$destination/$arch-osaca.txt"
}
build_reports x86_64 skylake-avx512 SKX
build_reports aarch64 neoverse-n1 N1
