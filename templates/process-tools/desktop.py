#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Stage unchanged X11 programs and their own libraries for native desktop acceptance.

This component is separate from the published runtime until source export and confinement
are complete. Extraction runs no package hooks; relocation never rewrites an executable.
"""
import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
from archive_inputs import GLIBC_SONAMES, REPO, ROOT, acquire, digest


PROGRAMS = ('Xvfb', 'xdotool', 'xauth', 'xkbcomp')


def assemble(cache, output, libraries):
    output.mkdir(parents=True, exist_ok=False)
    payload = output / 'payload'
    payload.mkdir()
    packages = json.loads((ROOT / 'desktop.lock.json').read_text())
    notices = output / 'notices'
    notices.mkdir()
    for item in packages:
        archive = acquire(dict(item, name=item['package']), cache)
        subprocess.run(['dpkg-deb', '--extract', str(archive), str(payload)], check=True)
        source = ROOT / item['notice']
        if digest(source) != item['noticeSha256']:
            raise ValueError('Desktop copyright differs from its reviewed bytes')
        shutil.copy2(source, notices / (item['package'] + '.copyright'))
    # The XKB utility package also carries unrelated GUI programs; retain only this component's executables.
    for path in (payload / 'usr/bin').iterdir():
        if path.name not in PROGRAMS:
            if not path.is_symlink() and not path.is_file():
                raise ValueError('Unexpected desktop executable directory')
            path.unlink()
    for path in (ROOT / 'notices/desktop').iterdir():
        if not path.name.endswith('.copyright'):
            shutil.copy2(path, notices / path.name)
    shutil.copy2(ROOT / 'desktop.lock.json', notices / 'desktop.lock.json')
    shutil.copy2(REPO / 'LICENSE.md', notices / 'pryx-LICENSE.md')
    base_notices = libraries.parent.parent / 'notices/libraries'
    if not base_notices.is_dir():
        raise ValueError('Desktop staging requires the reviewed base-library notices')
    shutil.copytree(base_notices, notices / 'base-libraries', symlinks=True)
    shutil.copytree(libraries, output / 'libraries', symlinks=True)
    binaries = output / 'bin'
    binaries.mkdir()
    for name in PROGRAMS:
        launcher = binaries / name
        launcher.write_text('#!/bin/sh\n# SPDX-License-Identifier: FSL-1.1-ALv2\nset -eu\n'
                            'component=$(CDPATH=\'\' cd -- "$(dirname -- "$0")/.." && pwd -P)\n'
                            'export LD_LIBRARY_PATH="$component/payload/usr/lib/x86_64-linux-gnu:'
                            '$component/payload/lib/x86_64-linux-gnu:$component/libraries"\n'
                            f'exec "$component/payload/usr/bin/{name}" "$@"\n')
        launcher.chmod(0o755)


def audit(root):
    minimum = json.loads((ROOT / 'compatibility.json').read_text())['desktopHostGlibcMinimum']
    allowed = tuple(map(int, minimum.split('.')))
    maximum = (0, 0)
    members = []
    libraries = [root / 'payload/usr/lib/x86_64-linux-gnu', root / 'payload/lib/x86_64-linux-gnu', root / 'libraries']
    missing = set()
    for path in sorted(root.rglob('*')):
        relative = str(path.relative_to(root))
        if path.name in GLIBC_SONAMES or path.name.startswith('ld-linux'):
            raise ValueError('A private glibc is forbidden in desktop tools')
        if path.is_symlink():
            if not path.resolve().is_relative_to(root):
                raise ValueError('Desktop link escapes its component')
            members.append(dict(path=relative, link=str(path.readlink())))
        elif path.is_file():
            with path.open('rb') as source:
                elf = source.read(4) == b'\x7fELF'
            if elf:
                version = subprocess.check_output(['readelf', '--version-info', '--wide', str(path)], text=True, stderr=subprocess.DEVNULL)
                if 'GLIBC_ABI_' in version:
                    raise ValueError('Desktop ELF has an unaccounted glibc ABI feature')
                required = [tuple(map(int, v.split('.'))) for v in re.findall(r'GLIBC_(\d+\.\d+(?:\.\d+)?)', version)]
                maximum = max([maximum, *required])
                if maximum > allowed:
                    raise ValueError(f'Desktop ELF exceeds its host glibc floor: {relative}')
                dynamic = subprocess.check_output(['readelf', '--dynamic', str(path)], text=True)
                for name in re.findall(r'\(NEEDED\).*\[([^]]+)\]', dynamic):
                    if name not in GLIBC_SONAMES and not any((directory / name).is_file() for directory in libraries):
                        missing.add(name)
            members.append(dict(path=relative, sha256=digest(path)))
    if missing:
        raise ValueError(f'Desktop libraries are incomplete: {sorted(missing)}')
    return dict(format=1, hostGlibcMinimum=minimum, maximumRequiredGlibc='.'.join(map(str, maximum)),
                files=members, scope='Staged component only; not published or announced')


def smoke(root):
    for name, flag in [('Xvfb', '-help'), ('xdotool', '--version'), ('xauth', '-V'), ('xkbcomp', '-version')]:
        subprocess.run([str(root / 'bin' / name), flag], check=True, timeout=10,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cache', type=Path)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--libraries', type=Path)
    parser.add_argument('--report', type=Path, required=True)
    parser.add_argument('--smoke-only', action='store_true')
    options = parser.parse_args()
    root = options.output.resolve()
    if not options.smoke_only:
        if options.cache is None or options.libraries is None:
            parser.error('Assembly needs the cache and verified base libraries')
        assemble(options.cache, root, options.libraries.resolve())
    report = audit(root)
    observed = os.confstr('CS_GNU_LIBC_VERSION')
    if not observed or tuple(map(int, observed.split()[-1].split('.'))) < tuple(map(int, report['hostGlibcMinimum'].split('.'))):
        raise ValueError('Desktop controls require a sufficiently new host glibc')
    report['observedHostGlibc'] = observed
    smoke(root)
    report['relocatedProgramControls'] = list(PROGRAMS)
    options.report.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({key: value for key, value in report.items() if key != 'files'}))


if __name__ == '__main__':
    main()
