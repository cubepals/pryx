#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Prepare immutable Alpine and unchanged pasta source for the network comparison.

The image stays a tar consumed by the maintained EROFS builder. Copyleft utilities remain
separate guest/host executables with their original sources and notices, never linked to pryx.
"""
import argparse
import json
from pathlib import Path
import subprocess
import tarfile

from inventory import ROOT, acquire


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, required=True)
    args = parser.parse_args()
    args.root.mkdir(parents=True, exist_ok=False, mode=0o700)
    cache = args.root / 'archives'
    inputs = json.loads((ROOT / 'network/inputs.json').read_text())
    for name, item in inputs.items():
        digest = acquire(item['url'], item['bytes'], item['sha256'], cache=cache)
        archive = cache / (digest + '.archive')
        if name == 'alpine':
            (args.root / 'alpine.tar.gz').hardlink_to(archive)
        elif name == 'pasta':
            destination = args.root / 'pasta-source'
            destination.mkdir()
            with tarfile.open(archive) as source:
                source.extractall(destination, filter='data')
            roots = list(destination.iterdir())
            if len(roots) != 1 or not roots[0].is_dir():
                raise ValueError('Unexpected upstream pasta source layout')
            subprocess.run(['make', '-C', str(roots[0]), '-j2', 'VERSION=' + item['version'], 'pasta'],
                           check=True, timeout=180)
            (args.root / 'pasta').symlink_to(roots[0] / 'pasta')
        else:
            raise ValueError('Unexpected network experiment input')
        print(json.dumps({'input': name, **item, 'verifiedSha256': digest}), flush=True)


if __name__ == '__main__':
    main()
