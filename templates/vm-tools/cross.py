#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Stage a signature-verified musl development sysroot without changing host packages.

This is a build input for the repository's pinned Rust musl target. It is never a native
process-tool libc replacement or a runtime prerequisite installed on the user's host.
"""
import argparse
import json
from pathlib import Path
import subprocess
import tempfile

from filesystem import bootstrap, local, source_inputs
from inventory import ROOT


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, required=True)
    parser.add_argument('--cache', type=Path, required=True)
    args = parser.parse_args()
    args.root = args.root.resolve()
    args.cache = args.cache.resolve()
    args.root.mkdir(mode=0o700, parents=True, exist_ok=False)
    catalog = json.loads((ROOT / 'filesystem-inputs.json').read_text())
    development = json.loads((ROOT / 'cross-inputs.json').read_text())
    runtime = next(item for item in catalog['packages'] if item['name'] == 'musl')
    if development['version'] != runtime['version'] or development['sourceCommit'] != runtime['sourceCommit']:
        raise ValueError('VM cross headers and libc must have identical source and package versions')
    source_inputs(catalog, args.cache, args.root)
    with tempfile.TemporaryDirectory(dir=args.root) as directory:
        verifier = Path(directory)
        bootstrap(local(catalog['bootstrap'], args.cache), verifier)
        packages = [local(item, args.cache) for item in [runtime, development]]
        sysroot = args.root / 'sysroot'
        sysroot.mkdir()
        subprocess.run([str(verifier / 'lib/ld-musl-x86_64.so.1'), '--library-path',
                        str(verifier / 'lib') + ':' + str(verifier / 'usr/lib'),
                        str(verifier / 'sbin/apk'), '--keys-dir', str(verifier / 'etc/apk/keys'),
                        '--no-network', 'extract', '--no-chown', '--destination', str(sysroot),
                        *map(str, packages)], check=True, timeout=60)
    print(json.dumps({'scope': 'Build-only musl sysroot; no host installation',
                      'runtime': runtime, 'development': development}), flush=True)


if __name__ == '__main__':
    main()
