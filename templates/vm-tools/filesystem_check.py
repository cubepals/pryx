#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Exercise the relocatable VM filesystem programs without root or a mount.

The real builders and checkers validate EROFS extraction and a populated ext4 image. This
does not replace the separate real-KVM boot test or claim production VM readiness.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tarfile
import tempfile

from filesystem import command


def run(root, program, *arguments, environment=None):
    return subprocess.run(command(root, program, *arguments), check=True, timeout=60,
                          env=environment, capture_output=True, text=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--component', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix='relocated-vm-filesystems-') as temporary:
        root = Path(temporary)
        tools = root / 'relocated/tools'
        shutil.copytree(args.component / 'tools', tools)
        manifest = json.loads((args.component / 'filesystem-manifest.json').read_text())
        for entry in manifest:
            path = tools.parent / entry['path']
            if path.is_symlink() or hashlib.sha256(path.read_bytes()).hexdigest() != entry['sha256']:
                raise ValueError('Relocated filesystem component differs from its inventory')
        loaded = {}
        for program in ['mkfs.erofs', 'fsck.erofs', 'mke2fs', 'e2fsck']:
            result = subprocess.run([str(tools / 'libraries/ld-musl-x86_64.so.1'), '--library-path',
                                     str(tools / 'libraries'), '--list', str(tools / 'helpers' / program)],
                                    check=True, capture_output=True, text=True, timeout=10)
            headers = subprocess.check_output(['readelf', '--wide', '--program-headers',
                                               str(tools / 'helpers' / program)], text=True, timeout=10)
            interpreter = re.search(r'\[Requesting program interpreter: (.+)\]', headers).group(1)
            loader_digest = hashlib.sha256((tools / 'libraries/ld-musl-x86_64.so.1').read_bytes()).digest()
            for library, path in re.findall(r'(\S+)\s+=>\s+(/\S+)', result.stdout):
                # musl prints its unchanged ELF interpreter name for its own libc
                # alias. That is the loader we explicitly executed, not a second
                # host library load. Require the packaged alias's identical bytes.
                if path == interpreter:
                    alias = tools / 'libraries' / library
                    if hashlib.sha256(alias.read_bytes()).digest() != loader_digest:
                        raise ValueError('Loader alias differs from the executed musl')
                else:
                    Path(path).relative_to(tools)
            loaded[program] = result.stdout
        source = root / 'input'
        source.mkdir()
        payload = b'pryx unchanged VM filesystem programs\n'
        (source / 'payload').write_bytes(payload)
        archive = root / 'input.tar'
        with tarfile.open(archive, 'w') as output:
            output.add(source / 'payload', arcname='payload')
        erofs = root / 'root.erofs'
        run(tools, 'mkfs.erofs', '--quiet', '--tar=f', erofs, archive)
        extracted = root / 'extracted'
        run(tools, 'fsck.erofs', '--extract=' + str(extracted), '--no-preserve-owner', erofs)
        if (extracted / 'payload').read_bytes() != payload:
            raise ValueError('EROFS round trip changed the file')
        disk = root / 'writes.ext4'
        with disk.open('xb') as output:
            output.truncate(128 * 1024 * 1024)
        environment = {**os.environ, 'MKE2FS_CONFIG': str(tools / 'config/mke2fs.conf')}
        run(tools, 'mke2fs', '-q', '-t', 'ext4', '-F', '-d', source, disk, environment=environment)
        checked = run(tools, 'e2fsck', '-fn', disk)
        report = {'scope': 'Relocated filesystem component only; not production VM lifecycle',
                  'uid': os.geteuid(), 'status': 'success', 'files': len(manifest),
                  'loaded': loaded, 'erofsBytes': erofs.stat().st_size,
                  'ext4Bytes': disk.stat().st_size, 'ext4Check': checked.stdout + checked.stderr}
        args.output.write_text(json.dumps(report, indent=2) + '\n')
        print(json.dumps(report), flush=True)


if __name__ == '__main__':
    main()
