#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Retain complete immutable Debian source inputs for the staged desktop programs.

Checksums cover the DSC, original archives and Debian changes; no build or package hook
runs. The source/version mapping comes from the exact binary packages' control metadata.
"""
import argparse
from email.parser import Parser
import json
from pathlib import Path
import shutil
from urllib.parse import urljoin, urlparse
from archive_inputs import REPO, ROOT, acquire, digest


def collect(cache, output, *, binary_lock='desktop.lock.json', source_lock='desktop-sources.lock.json', licence_directory='notices/desktop'):
    binaries = json.loads((ROOT / binary_lock).read_text())
    catalog = json.loads((ROOT / source_lock).read_text())
    expected = {(p['sourcePackage'], p['sourceVersion']) for p in binaries}
    actual = {(p['sourcePackage'], p['sourceVersion']) for p in catalog}
    if actual != expected:
        raise ValueError('Desktop corresponding-source coverage differs from its binary inputs')
    by_url = {p['url']: p for p in catalog}
    if len(by_url) != len(catalog):
        raise ValueError('Duplicate desktop source URL')
    inputs = {}
    for item in catalog:
        source = acquire(item, cache)
        if source.stat().st_size != item['bytes']:
            raise ValueError('Desktop source archive size differs from its lock')
        inputs[item['url']] = source
    descriptors = {}
    referenced = set()
    for item in catalog:
        if not urlparse(item['url']).path.endswith('.dsc'):
            continue
        text = inputs[item['url']].read_text()
        if text.startswith('-----BEGIN PGP SIGNED MESSAGE-----'):
            text = text.split('\n\n', 1)[1]
        fields = Parser().parsestr(text.split('-----BEGIN PGP SIGNATURE-----', 1)[0])
        identity = (fields['Source'], fields['Version'])
        if identity != (item['sourcePackage'], item['sourceVersion']) or identity in descriptors:
            raise ValueError('Desktop DSC identity differs from the binary source mapping')
        descriptors[identity] = item['url']
        referenced.add(item['url'])
        checksums = fields['Checksums-Sha256']
        if not checksums or not checksums.strip():
            raise ValueError('Desktop DSC has no source checksum list')
        for line in checksums.splitlines():
            if not line.strip():
                continue
            sha256, size, name = line.split()
            if Path(name).name != name or name in {'.', '..'}:
                raise ValueError('Desktop DSC archive name escapes its directory')
            url = urljoin(item['url'], name)
            archive = by_url.get(url)
            if archive is None or archive['sha256'] != sha256 or archive['bytes'] != int(size):
                raise ValueError('Desktop DSC source checksum differs from its immutable input')
            if (archive['sourcePackage'], archive['sourceVersion']) != identity:
                raise ValueError('Desktop source archive has a different source identity')
            referenced.add(url)
    if set(descriptors) != expected or referenced != set(by_url):
        raise ValueError('Desktop source export must retain exactly every complete DSC input set')
    output.mkdir(parents=True, exist_ok=False)
    files = []
    for item in catalog:
        source = inputs[item['url']]
        destination = output / 'archives' / item['sha256'] / source.name
        destination.parent.mkdir(parents=True)
        shutil.copy2(source, destination)
        files.append(dict(path=str(destination.relative_to(output)), sha256=digest(destination)))
    for name in [binary_lock, source_lock]:
        shutil.copy2(ROOT / name, output / name)
        files.append(dict(path=name, sha256=digest(output / name)))
    notices = output / 'notices'
    notices.mkdir()
    for item in binaries:
        notice = ROOT / item['notice']
        if digest(notice) != item['noticeSha256']:
            raise ValueError('Desktop source copyright differs from its reviewed bytes')
        shutil.copy2(notice, notices / (item['package'] + '.copyright'))
    for path in (ROOT / licence_directory).iterdir():
        if not path.name.endswith('.copyright'):
            shutil.copy2(path, notices / path.name)
    shutil.copy2(REPO / 'LICENSE.md', notices / 'pryx-LICENSE.md')
    files.extend(dict(path=str(p.relative_to(output)), sha256=digest(p)) for p in sorted(notices.iterdir()))
    report = dict(format=1, sourceVersions=len(descriptors), archiveInputs=len(catalog),
                  binaryPackages=len(binaries), files=files,
                  scope='Corresponding-source staging only; no runtime publication or desktop availability')
    (output / 'manifest.json').write_text(json.dumps(report, indent=2) + '\n')
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cache', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--full', action='store_true', help='Validate the separately staged complete desktop inputs')
    options = parser.parse_args()
    report = collect(options.cache, options.output,
                     **dict(binary_lock='desktop-full.lock.json', source_lock='desktop-full-sources.lock.json', licence_directory='notices/desktop-full') if options.full else {})
    print(json.dumps({k: v for k, v in report.items() if k != 'files'}))


if __name__ == '__main__':
    main()
