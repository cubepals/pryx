#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Retain original registry sources and notices for the VM helper's actual Linux dependency graph.

Only normal/build edges of the resolved musl graph are selected. Cargo.lock authenticates each
original crate archive; archive members are read as data and never extracted onto host paths.
"""
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tarfile
import tomllib

from inventory import ROOT


def collect(runtime, sources):
    manifest = ROOT.parent.parent / 'vm-image/Cargo.toml'
    metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--locked', '--manifest-path', str(manifest),
        '--format-version', '1', '--filter-platform', 'x86_64-unknown-linux-musl'], timeout=60))
    nodes = {node['id']: node for node in metadata['resolve']['nodes']}
    selected, pending = set(), [metadata['resolve']['root']]
    while pending:
        identity = pending.pop()
        if identity in selected:
            continue
        selected.add(identity)
        pending.extend(dep['pkg'] for dep in nodes[identity]['deps']
                       if any(kind['kind'] != 'dev' for kind in dep['dep_kinds']))
    lock = tomllib.loads((manifest.parent / 'Cargo.lock').read_text())
    checksums = {(item['name'], item['version'], item.get('source')): item['checksum']
                 for item in lock['package'] if 'checksum' in item}
    inventory = []
    for package in metadata['packages']:
        if package['id'] not in selected or package['source'] is None:
            continue
        identity = f"{package['name']}-{package['version']}"
        root = Path(package['manifest_path']).parent
        archive = root.parent.parent.parent / 'cache' / root.parent.name / f'{identity}.crate'
        with archive.open('rb') as source:
            digest = hashlib.file_digest(source, 'sha256').hexdigest()
        if digest != checksums[(package['name'], package['version'], package['source'])]:
            raise ValueError('VM dependency differs from its Cargo.lock archive')
        target = sources / 'rust' / archive.name
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(archive, target)
        names = []
        with tarfile.open(archive) as source:
            prefix = identity + '/'
            for member in source:
                if not member.name.startswith(prefix):
                    raise ValueError('Unexpected Rust source archive prefix')
                relative = Path(member.name.removeprefix(prefix))
                notice = relative.name.upper().startswith(('LICENSE', 'LICENCE', 'COPYING', 'NOTICE', 'COPYRIGHT'))
                metadata_file = len(relative.parts) == 1 and relative.name in ('Cargo.toml', 'README.md', 'README')
                declared = package.get('license_file') == str(relative)
                if not member.isfile() or not (notice or metadata_file or declared):
                    continue
                if relative.is_absolute() or '..' in relative.parts or member.size > 2 * 1024 * 1024:
                    raise ValueError('Unsafe Rust primary-notice member')
                destination = runtime / 'notices/rust' / identity / relative
                destination.parent.mkdir(parents=True, exist_ok=True)
                destination.write_bytes(source.extractfile(member).read(member.size))
                names.append(str(relative))
        inventory.append({'name': package['name'], 'version': package['version'], 'licence': package['license'],
                          'repository': package['repository'], 'sourceSha256': digest, 'retained': sorted(names)})
    directory = runtime / 'notices/rust'
    directory.mkdir(parents=True, exist_ok=True)
    (directory / 'inventory.json').write_text(json.dumps(inventory, indent=2) + '\n')
