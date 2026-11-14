#!/usr/bin/env bash
# SPDX-License-Identifier: FSL-1.1-ALv2
# Assemble the exact VM distribution proved by the VM workflow, without starting KVM,
# changing host policy, accessing signing material or publishing anything. Only its
# descriptor enters the signed host installation; large runtime/source assets stay lazy.
set -euo pipefail
: "${RUNNER_TEMP:?Provide a private VM build directory}"
: "${PRYX_VM_RELEASE_BASE_URL:?Provide the exact release asset base URL}"
pryx_vm_destination=${1:?Name the release asset directory}
pryx_vm_recipe="$PWD/templates/vm-tools/build.sh"
export CGO_ENABLED=0 GOTOOLCHAIN=local
for pryx_vm_phase in inventory tools inspect filesystems; do
  bash "$pryx_vm_recipe" "$pryx_vm_phase"
done
(cd templates/vm-tools && bash "$pryx_vm_recipe" configurator)
(cd templates/vm-tools/network && bash "$pryx_vm_recipe" network)
rustup toolchain install
for pryx_vm_phase in bootstrap sources bundle; do
  bash "$pryx_vm_recipe" "$pryx_vm_phase"
done
mkdir -p "$pryx_vm_destination"
cp "$RUNNER_TEMP/pryx-vm-linux-x64.tar.gz" "$RUNNER_TEMP/pryx-vm-sources.tar.gz" "$pryx_vm_destination/"
