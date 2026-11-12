#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Extract reviewed X11 programs for the process-policy design test, without installing them.

This fixture is separate from the published runtime. It runs no package hooks and carries
only digest-locked K18 packages, their notices, and K23's already verified shared libraries.
"""
import argparse
import json
from pathlib import Path
import shutil
import subprocess
import build


PACKAGES = {
    'xvfb', 'xserver-common', 'xdotool', 'libxdo3', 'libxfont2', 'libfontenc1',
    'libxkbfile1', 'libxtst6', 'x11-xkb-utils', 'xkb-data', 'xauth',
}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cache', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--libraries', type=Path, required=True)
    options = parser.parse_args()
    libraries = options.libraries.resolve()
    forbidden = {'libc.so.6', 'libm.so.6', 'libpthread.so.0', 'libdl.so.2', 'librt.so.1', 'libresolv.so.2'}
    if any(p.name in forbidden or p.name.startswith('ld-linux') for p in libraries.rglob('*')):
        raise ValueError('A private glibc is forbidden in the desktop fixture')
    output = options.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    catalog = json.loads((build.REPO / 'templates/native-tools/debian.lock.json').read_text())
    selected = [p for p in catalog if p['package'] in PACKAGES]
    if {p['package'] for p in selected} != PACKAGES:
        raise ValueError('Incomplete reviewed X11 fixture inputs')
    for item in selected:
        archive = build.acquire(dict(item, name=item['package']), options.cache)
        subprocess.run(['dpkg-deb', '--extract', str(archive), str(output)], check=True)
        notice = build.REPO / 'templates/native-tools/notices' / (item['package'] + '.txt')
        target = output / 'notices' / notice.name
        target.parent.mkdir(exist_ok=True)
        shutil.copy2(notice, target)
    shutil.copytree(libraries, output / 'libraries', symlinks=True)
    programs = {}
    for name in ['Xvfb', 'xdotool']:
        path = output / 'usr/bin' / name
        programs[name] = dict(path=str(path.relative_to(output)), sha256=build.digest(path))
    record = dict(format=1, packages=selected, programs=programs,
                  scope='Design fixture only; no native desktop availability claim')
    (output / 'fixture.json').write_text(json.dumps(record, indent=2) + '\n')
    print(json.dumps(record))


if __name__ == '__main__':
    main()
