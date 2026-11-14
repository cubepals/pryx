#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Select nft's verified ELF closure for the trusted read-only guest bootstrap.

Only inventoried component files enter the image. Image libraries and host loader paths
never participate; readelf reads dependency names from the unchanged upstream programs.
"""
from collections import deque
import hashlib
import json
from pathlib import PurePosixPath
import re
import subprocess
import tarfile

from filesystem import read_only_manifest


def add(archive, component, metadata):
    root = PurePosixPath(metadata['vmFirewallDirectory'])
    program = PurePosixPath(metadata['vmFirewallProgram'])
    loader = PurePosixPath(metadata['vmFirewallLoader'])
    for path in [root, program, loader]:
        if path.is_absolute() or '..' in path.parts or not path.parts:
            raise ValueError('Invalid trusted firewall path')
    manifest = component / 'filesystem-manifest.json'
    entries = {entry['path']: entry for entry in json.loads(manifest.read_text())}
    for directory in sorted({root, root / program.parent, root / loader.parent}):
        member = tarfile.TarInfo(str(directory))
        member.type, member.mode = tarfile.DIRTYPE, 0o555
        archive.addfile(member)
    pending = deque([program, loader])
    seen = set()
    while pending:
        relative = pending.popleft()
        if relative in seen:
            continue
        seen.add(relative)
        name = str(PurePosixPath('tools') / relative)
        entry = entries[name]
        path = component / name
        data = path.read_bytes()
        if path.is_symlink() or len(data) != entry['bytes'] or hashlib.sha256(data).hexdigest() != entry['sha256']:
            raise ValueError('Trusted firewall differs from its verified component')
        dynamic = subprocess.check_output(['readelf', '--wide', '--dynamic', str(path)], text=True, timeout=10)
        for library in re.findall(r'\(NEEDED\).*\[([^\]]+)\]', dynamic):
            if PurePosixPath(library).name != library:
                raise ValueError('Firewall dependency must be one library name')
            pending.append(loader.parent / library)
        member = tarfile.TarInfo(str(root / relative))
        member.mode, member.size = 0o555, len(data)
        with path.open('rb') as source:
            archive.addfile(member, source)
    return hashlib.sha256(read_only_manifest(manifest)).hexdigest()
