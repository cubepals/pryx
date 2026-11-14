#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Prepare the pinned separate GNU compiler without host installation or library overrides.

Verify the primary archive before extraction. The publisher's relocation script runs unchanged;
only catalogue entries for absent auxiliary files are omitted and recorded. The SDK's unsafe
host-library guard remains enabled. This directory never enters the runtime distribution.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tarfile

from inventory import ROOT, acquire


def prepare(root, cache):
    inputs = json.loads((ROOT / 'monitor-inputs.json').read_text())['sdk']
    digest = acquire(inputs['url'], inputs['bytes'], inputs['sha256'], cache=cache)
    root.mkdir(parents=True, mode=0o700, exist_ok=False)
    with tarfile.open(cache / (digest + '.archive')) as archive:
        archive.extractall(root, filter='data')
    sdk = root / inputs['name']
    catalogue = sdk / 'share/buildroot/sdk-relocs'
    original = catalogue.read_bytes()
    present, absent = [], []
    for entry in original.decode().splitlines():
        path = sdk / entry
        if path.is_absolute() and not path.resolve().is_relative_to(sdk.resolve()):
            raise ValueError('SDK relocation entry escapes its private root')
        (present if path.exists() else absent).append(entry)
    catalogue.write_text('\n'.join(present) + '\n')
    subprocess.run([str((sdk / 'relocate-sdk.sh').resolve())], cwd=sdk, check=True, timeout=120)
    compiler = sdk / 'bin/x86_64-buildroot-linux-gnu-gcc'
    version = subprocess.check_output([str(compiler), '--version'], text=True, timeout=10).splitlines()[0]
    sysroot = Path(subprocess.check_output([str(compiler), '-print-sysroot'], text=True, timeout=10).strip())
    if not sysroot.resolve().is_relative_to(sdk.resolve()) or not (sysroot / 'usr/lib/libc.a').is_file():
        raise ValueError('Pinned GNU compiler did not resolve its private static libc')
    receipt = {'sdk': inputs, 'compilerVersion': version, 'sysroot': str(sysroot),
               'originalRelocationCatalogueSha256': hashlib.sha256(original).hexdigest(),
               'absentAuxiliaryEntries': absent, 'existingEntriesRelocated': len(present)}
    (root / 'sdk.json').write_text(json.dumps(receipt, indent=2) + '\n')
    print(json.dumps(receipt), flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, required=True)
    parser.add_argument('--cache', type=Path, required=True)
    args = parser.parse_args()
    prepare(args.root.resolve(), args.cache.resolve())


if __name__ == '__main__':
    main()
