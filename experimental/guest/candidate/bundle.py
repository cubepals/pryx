#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Derive a private candidate archive from the fully verified, staged stock runtime.

The original host launcher, policy and kernel retain their identities. Four new
guest roles stay in the trusted EROFS image; installed code and the release's
descriptor never change. Only the recomputed descriptor is evidence.
"""
import argparse
import importlib.util
import json
from pathlib import Path
import shutil
import sys

VM = Path(__file__).resolve().parents[3] / 'templates/vm-tools'
sys.path.insert(0, str(VM))
spec = importlib.util.spec_from_file_location('pryx_stock_vm_bundle', VM / 'bundle.py')
stock = importlib.util.module_from_spec(spec)
spec.loader.exec_module(stock)


def build(runtime, bootstrap, configurator, output):
    stock.package.verify(runtime)
    existing = json.loads((runtime / 'bootstrap/bootstrap.json').read_text())
    changed = json.loads((bootstrap / 'bootstrap.json').read_text())
    for key in ['agentSha256', 'filesystemManifestSha256', 'networkHelperSha256',
                'packetForwarderSha256', 'metadata', 'networkProbe']:
        if changed[key] != existing[key]:
            raise ValueError(f'Candidate changed a fixed runtime identity: {key}')
    if (changed['networkProbe'] is not None or
            stock.package.digest(runtime / 'bin/vm-launcher') != changed['agentSha256'] or
            stock.package.digest(bootstrap / 'bootstrap.erofs') != changed['imageSha256']):
        raise ValueError('Candidate lifecycle requires a verified service, launcher and whole production image')
    output.mkdir(mode=0o700, exist_ok=False)
    root = output / 'runtime'
    stock.copy_tree(runtime, root)
    (root / 'manifest.json').unlink()
    for name, source in [('bootstrap/bootstrap.erofs', bootstrap / 'bootstrap.erofs'),
                         ('bootstrap/bootstrap.json', bootstrap / 'bootstrap.json'), ('bin/vm-config', configurator)]:
        target = root / name
        mode = target.stat().st_mode & 0o777
        target.unlink()
        shutil.copyfile(source, target)
        target.chmod(mode)
    stock.inventory(root)
    original = {e['path']: e for e in json.loads((runtime / 'manifest.json').read_text())['files']}
    revised = {e['path']: e for e in json.loads((root / 'manifest.json').read_text())['files']}
    expected = {'bin/vm-config', 'bootstrap/bootstrap.erofs', 'bootstrap/bootstrap.json'}
    if set(original) != set(revised) or {p for p in revised if revised[p] != original[p]} != expected:
        raise ValueError('The private candidate must differ at exactly three runtime paths')
    result = stock.package.package(root, output / 'runtime.tar.gz', 'http://127.0.0.1/runtime')
    (output / 'descriptor.json').write_text(json.dumps(result, indent=2) + '\n')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ['runtime', 'bootstrap', 'configurator', 'output']:
        parser.add_argument(name, type=Path)
    args = parser.parse_args()
    build(args.runtime.resolve(strict=True), args.bootstrap.resolve(strict=True),
          args.configurator.resolve(strict=True), args.output.resolve())
