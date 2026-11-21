#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Build two lab kernels differing in the serial timestamp setting.

The VM workflow already retained the pinned Linux source and published config.
The outer timeout covers both compilations together, including configuration.
Publish neither kernel. Compare only the rebuilt control against its sibling.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile

VM_TOOLS = Path(__file__).resolve().parents[2] / 'templates/vm-tools'
sys.path.insert(0, str(VM_TOOLS))
from kernel import selected


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('staged', type=Path)
    parser.add_argument('dest', type=Path)
    args = parser.parse_args()
    stage, dest = args.staged.resolve(), args.dest.resolve()
    dest.mkdir(mode=0o700)
    linux = json.loads((VM_TOOLS / 'kernel-source.json').read_text())
    selection = json.loads((VM_TOOLS / 'kernel-selection.json').read_text())
    inspection = json.loads((stage / 'vm-archive-inspection.json').read_text())
    config = selected(selection, inspection, stage / 'vm-selected-files', 'configuration')
    archive = stage / 'vm-corresponding-sources/sources' / Path(linux['url']).name
    with archive.open('rb') as archive_file:
        if hashlib.file_digest(archive_file, 'sha256').hexdigest() != linux['sha256']:
            raise ValueError('Kernel source hash differs from the committed selection')
    with tarfile.open(archive) as sources:
        sources.extractall(dest, filter='data')
    src = dest / f"linux-{linux['version']}"
    build = dest / 'obj'
    build.mkdir()
    (build / '.config').write_text(config)
    jobs = len(os.sched_getaffinity(0))
    env = {**os.environ, 'KBUILD_BUILD_TIMESTAMP': '1970-01-01 00:00:00 UTC',
           'KBUILD_BUILD_USER': 'lab', 'KBUILD_BUILD_HOST': 'lab', 'KBUILD_BUILD_VERSION': '1'}
    make = ['make', '-s', '-C', src, f'O={build}', f'-j{jobs}']
    # The same published config and pinned unpatched source form both cohorts.
    # Distribution signing files can be external to the kernel source.
    config_cmd = [src / 'scripts/config', '--file', build / '.config']
    subprocess.run([*config_cmd, '--set-str', 'SYSTEM_TRUSTED_KEYS', '',
                    '--set-str', 'SYSTEM_REVOCATION_KEYS', ''], check=True)
    for label in ['control', 'no-print-time']:
        if label == 'no-print-time':
            subprocess.run([*config_cmd, '--disable', 'PRINTK_TIME'], check=True)
        subprocess.run([*make, 'olddefconfig'], env=env, check=True)
        actual = (build / '.config').read_text().splitlines()
        want = 'CONFIG_PRINTK_TIME=y' if label == 'control' else '# CONFIG_PRINTK_TIME is not set'
        if want not in actual:
            raise ValueError(f'Timestamp setting unavailable in kernel {label}')
        missing = [k for k in selection['requiredConfiguration'] if f'{k}=y' not in actual]
        if missing:
            raise ValueError(f'Rebuilt kernel lacks required capabilities: {missing}')
        shutil.copyfile(build / '.config', dest / f'{label}.config')
        subprocess.run([*make, 'vmlinux'], env=env, check=True)
        shutil.copyfile(build / 'vmlinux', dest / f'{label}.elf')


if __name__ == '__main__':
    main()
