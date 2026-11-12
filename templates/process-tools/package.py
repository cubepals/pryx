#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Package an inventoried upstream directory without changing any runtime file.

The small descriptor belongs in the signed base installation. The separate large archive
is acquired only when a sandbox asks for a tool; its digest and expansion bounds are pinned.
"""
import argparse
import gzip
import hashlib
import json
from pathlib import Path
import tarfile


def digest(path):
    with path.open('rb') as source:
        return hashlib.file_digest(source, 'sha256').hexdigest()


def verify(root):
    expected = {'manifest.json'}
    inventory = json.loads((root / 'manifest.json').read_text())
    for entry in inventory['files']:
        relative = Path(entry['path'])
        path = root / relative
        if relative.is_absolute() or '..' in relative.parts or str(relative) in expected:
            raise ValueError('Invalid or repeated tool inventory path')
        if not path.resolve().is_relative_to(root):
            raise ValueError('Tool inventory escapes its root')
        expected.add(str(relative))
        if 'link' in entry:
            if not path.is_symlink() or str(path.readlink()) != entry['link']:
                raise ValueError(f"Tool inventory link changed: {relative}")
        elif path.is_symlink() or not path.is_file() or digest(path) != entry['sha256']:
            raise ValueError(f"Tool inventory file changed: {relative}")
    actual = set()
    for path in root.rglob('*'):
        if path.is_symlink() or path.is_file():
            actual.add(str(path.relative_to(root)))
        elif not path.is_dir():
            raise ValueError('Unsupported tool directory entry')
    if actual != expected:
        raise ValueError('Tool directory differs from its inventory')


def package(root, archive, url):
    root = root.resolve()
    if archive.resolve().is_relative_to(root):
        raise ValueError('Archive output must be outside the tool directory')
    verify(root)
    count, expanded = 0, 0
    with archive.open('wb') as target:
        with gzip.GzipFile(filename='', fileobj=target, mode='wb', mtime=0, compresslevel=6) as compressed:
            with tarfile.open(fileobj=compressed, mode='w|', format=tarfile.PAX_FORMAT) as output:
                for path in sorted(root.rglob('*')):
                    info = output.gettarinfo(str(path), arcname=str(path.relative_to(root)))
                    info.uid = info.gid = info.mtime = 0
                    info.uname = info.gname = ''
                    count += 1
                    expanded += info.size
                    if info.isfile():
                        with path.open('rb') as source:
                            output.addfile(info, source)
                    else:
                        output.addfile(info)
    return dict(url=url, sha256=digest(archive), manifest_sha256=digest(root / 'manifest.json'),
                bytes=archive.stat().st_size, unpacked_bytes=expanded, entries=count)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, required=True)
    parser.add_argument('--archive', type=Path, required=True)
    parser.add_argument('--descriptor', type=Path, required=True)
    parser.add_argument('--url', required=True)
    options = parser.parse_args()
    if options.descriptor.resolve().is_relative_to(options.root.resolve()):
        raise ValueError('Descriptor output must be outside the tool directory')
    result = package(options.root, options.archive, options.url)
    options.descriptor.write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result))


if __name__ == '__main__':
    main()
