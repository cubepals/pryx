#!/bin/bash
# SPDX-License-Identifier: FSL-1.1-ALv2
# Run deploy/vm-installed-acceptance.ts as root against the server preset this disposable runner
# installed with sudo, on the node configuration exactly as the install wrote it. The credential
# is the one the install wrote for this user. Changes nothing on the host.
set -euo pipefail
: "${PRYX_PYTHON:?Name the Python that has the installed SDK}"
test "$(id -u)" -ne 0
failed() {
  echo 'Root VM acceptance failed; the node and VM worker journals end:' >&2
  sudo journalctl -u pryxd -u pryxd-vm-worker --no-pager -n 400 | tail -c 30000 >&2 || true
}
trap failed ERR
PRYX_API_KEY=$(pryx config key)
PRYX_API_URL=$(python3 -c 'import json, pathlib; print(json.loads((pathlib.Path.home() / ".config/pryx/client.json").read_text())["url"])')
PRYX_VM_INSTALLATION=server
export PRYX_API_KEY PRYX_API_URL PRYX_VM_INSTALLATION
sudo --preserve-env=PRYX_VM_INSTALLATION,PRYX_API_URL,PRYX_API_KEY,PRYX_PYTHON \
  "$(command -v bun)" deploy/vm-installed-acceptance.ts
echo 'root_installed_vm_acceptance=passed'
