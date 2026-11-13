#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Run real KVM guest-init acceptance against the actual static agent and pinned OCI images.

This reuses the published-input verifier and existing Cargo test system. It does not announce
VM placement, prove production confinement or substitute a fixture for the runtime backend.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess

from inventory import ROOT
from kernel import selected


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--inspection', type=Path, required=True)
    parser.add_argument('--staged', type=Path, required=True)
    parser.add_argument('--configurator', type=Path, required=True)
    parser.add_argument('--client', type=Path, required=True)
    parser.add_argument('--proxy', type=Path, required=True)
    parser.add_argument('--bootstrap', type=Path, required=True)
    parser.add_argument('--filesystem-component', type=Path, required=True)
    parser.add_argument('--work', type=Path, required=True)
    args = parser.parse_args()
    selection = json.loads((ROOT / 'kernel-selection.json').read_text())
    inspection = json.loads(args.inspection.read_text())
    monitor = selected(selection, inspection, args.staged, 'monitor')
    kernel = selected(selection, inspection, args.staged, 'kernel')
    monitor.chmod(0o700)
    bootstrap = json.loads((args.bootstrap / 'bootstrap.json').read_text())
    if hashlib.sha256((args.bootstrap / 'bootstrap.erofs').read_bytes()).hexdigest() != bootstrap['imageSha256']:
        raise ValueError('Trusted bootstrap bytes changed after construction')
    environment = os.environ.copy()
    environment.update({
        'PRYX_VM_MONITOR': str(monitor.resolve()), 'PRYX_VM_KERNEL': str(kernel.resolve()),
        'PRYX_VM_BOOTSTRAP': str(args.bootstrap.resolve()),
        'PRYX_VM_CONFIGURATOR': str(args.configurator.resolve()),
        'PRYX_VM_AGENT_CLIENT': str(args.client.resolve()),
        'PRYX_VM_NETWORK_PROXY': str(args.proxy.resolve()),
        'PRYX_VM_FILESYSTEM_COMPONENT': str(args.filesystem_component.resolve()),
        'PRYX_VM_FILESYSTEM_MANIFEST': hashlib.sha256(
            (args.filesystem_component / 'filesystem-manifest.json').read_bytes()).hexdigest(),
        'PRYX_VM_GUEST_WORK': str(args.work.resolve()),
    })
    subprocess.run(['cargo', 'test', '--locked', '--manifest-path', str(ROOT.parent.parent / 'vm-image/Cargo.toml'),
                    '--test', 'guest', '--', '--ignored', '--nocapture'], env=environment, check=True, timeout=360)


if __name__ == '__main__':
    main()
