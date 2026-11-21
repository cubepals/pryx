#!/bin/bash
# SPDX-License-Identifier: FSL-1.1-ALv2
# Prepare the pinned kernel's host programs, then build both variants under one outer deadline.
set -euo pipefail
: "${RUNNER_TEMP:?private staged VM inputs}"
work="$RUNNER_TEMP/lab-kernels"
phase="$work/build-phase.txt"
printf '%s\n' 'installing external kernel toolchain dependencies' > "$phase"
packages=(libelf-dev bc bison flex make gcc binutils pkgconf)
sudo --non-interactive apt-get -o Acquire::Retries=2 update -qq
sudo --non-interactive apt-get -o Acquire::Retries=2 install -y --no-install-recommends "${packages[@]}"
dpkg-query -W -f='${binary:Package} ${Version}\n' "${packages[@]}" > "$work/build-packages.txt"
printf '%s\n' 'checking host objtool ELF headers and linker' > "$phase"
printf '#include <gelf.h>\nint main(void){return elf_version(EV_CURRENT)==EV_NONE;}\n' | \
  cc -Wall -Wextra -Werror -x c - -lelf -o "$work/check-libelf"
"$work/check-libelf"
if [[ ! -d "$RUNNER_TEMP/lab-inits/stamped-root" ]]; then
  printf '%s\n' 'staging the same stamped stock guest for both kernels' > "$phase"
  bash experimental/guest/prove.sh --stamp-only
fi
printf '%s\n' 'verifying source and published configuration' > "$phase"
exec python3 experimental/guest/kernel_pair.py "$RUNNER_TEMP" "$work"
