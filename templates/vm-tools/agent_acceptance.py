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
    parser.add_argument('--host-launcher', type=Path, required=True)
    parser.add_argument('--proxy', type=Path, required=True)
    parser.add_argument('--bootstrap', type=Path, required=True)
    parser.add_argument('--filesystem-component', type=Path, required=True)
    parser.add_argument('--work', type=Path, required=True)
    parser.add_argument('--measure-boot', action='store_true')
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
        'PRYX_VM_HOST_LAUNCHER': str(args.host_launcher.resolve(strict=True)),
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
    if args.measure_boot:
        config = selected(selection, inspection, args.staged, 'configuration')
        config_sha = hashlib.sha256(config.encode()).hexdigest()
        if config_sha != selection['configuration']['sha256']:
            raise ValueError('Published kernel configuration differs from its pin')
        source = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
        if os.environ.get('GITHUB_SHA', source) != source:
            raise ValueError('Boot measurements require the dispatched source')
        environment['PRYX_VM_BOOT_BENCHMARK'] = json.dumps({
            'source': source,
            'kernelSha256': selection['kernel']['sha256'],
            'configurationSha256': config_sha,
            'monitorSha256': selection['monitor']['sha256'],
            'pageReportingBuiltIn': 'CONFIG_PAGE_REPORTING=y' in config.splitlines(),
            'balloonBuiltIn': 'CONFIG_VIRTIO_BALLOON=y' in config.splitlines(),
        })
    subprocess.run(['cargo', 'test', '--locked', '--manifest-path', str(ROOT.parent.parent / 'vm-image/Cargo.toml'),
                    '--test', 'guest', '--', '--ignored', '--nocapture'], env=environment, check=True,
                   timeout=540 if args.measure_boot else 360)


if __name__ == '__main__':
    main()
