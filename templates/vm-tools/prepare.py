#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Prepare pinned build/filesystem tools for the hosted kernel experiment.

This private fixture installation requires no host package changes. It does not install the
product, build a kernel or prepare the production VM bundle. Every archive is verified before
extraction; complete packaged licences remain beside the separate tools.
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
    inputs = json.loads((ROOT / 'spike-inputs.json').read_text())
    for name, item in inputs.items():
        digest = acquire(item['url'], item['bytes'], item['sha256'], cache=cache)
        archive = cache / (digest + '.archive')
        destination = args.root / name
        destination.mkdir()
        if name == 'go':
            with tarfile.open(archive) as source:
                source.extractall(destination, filter='data')
        elif name == 'erofs':
            subprocess.run(['dpkg-deb', '--extract', str(archive), str(destination)], check=True)
        else:
            raise ValueError('Unknown pinned experiment tool')
        archive.unlink()
        print(json.dumps({'tool': name, **item, 'verifiedSha256': digest}), flush=True)


if __name__ == '__main__':
    main()
