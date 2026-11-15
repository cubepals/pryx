#!/bin/sh
# SPDX-License-Identifier: FSL-1.1-ALv2
# Compiles the process tier's cgroup programs into the object pryxd embeds (src/bpf/object.rs),
# with Ubuntu 24.04's clang 18, llvm-strip 18 and libbpf-dev 1.3 headers (docs/decisions.md, K31).
# CI builds it again and requires the same bytes: run this and commit network.bpf.o with a change.
set -eu
cd "$(dirname "$0")"
multiarch=$(gcc -print-multiarch)
"${CLANG:-clang-18}" -target bpfel -mcpu=v3 -O2 -g -Wall -Wextra -Werror -fno-ident \
  -ffile-prefix-map="$PWD"=. -idirafter "/usr/include/$multiarch" -c network.bpf.c -o network.bpf.o.new
# DWARF goes; BTF, which the maps and the kernel's verifier log need, stays.
"${LLVM_STRIP:-llvm-strip-18}" --strip-debug network.bpf.o.new
mv network.bpf.o.new network.bpf.o
