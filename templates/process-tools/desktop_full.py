#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Stage a complete, unchanged desktop/SSH dependency closure separately from publication.

Only selected ELF files and their original aliases enter the component. Non-executable
runtime data remain unchanged; package hooks, private glibc and executable rewriting are forbidden.
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import tempfile
from archive_inputs import REPO, ROOT, acquire, copy_entry, digest
import desktop


def inputs():
    return json.loads((ROOT / 'desktop-full.lock.json').read_text())


def configuration():
    return json.loads((ROOT / 'desktop-full.json').read_text())


def library_paths(root):
    directories = {str(Path(path).parent) for item in inputs() for path in item['elfs'] if '.so' in Path(path).name}
    return [root / 'payload' / path for path in sorted(directories)]


def audit(root):
    return desktop.audit(root, configuration()['hostGlibcMinimum'], library_paths(root))


def assemble(cache, output):
    packages = inputs()
    config = configuration()
    output.mkdir(parents=True, exist_ok=False)
    payload = output / 'payload'
    payload.mkdir()
    notices = output / 'notices'
    notices.mkdir()
    for item in packages:
        if item['package'] in {'libc6', 'libc-bin'}:
            raise ValueError('A private glibc is forbidden in the desktop component')
        archive = acquire(dict(item, name=item['package']), cache)
        source = subprocess.check_output(['dpkg-deb', '--field', str(archive), 'Source'], text=True).strip() or item['package']
        version = subprocess.check_output(['dpkg-deb', '--field', str(archive), 'Version'], text=True).strip()
        if ' (' in source:
            source, version = source[:-1].split(' (')
        if (source, version) != (item['sourcePackage'], item['sourceVersion']):
            raise ValueError('Desktop binary source identity differs from its corresponding-source lock')
        with tempfile.TemporaryDirectory(dir=cache) as directory:
            package = Path(directory)
            subprocess.run(['dpkg-deb', '--extract', str(archive), str(package)], check=True)
            wanted = {(package / path).resolve() for path in item['elfs']}
            for path in sorted(package.rglob('*')):
                relative = path.relative_to(package)
                if str(relative).startswith(('usr/share/doc/', 'usr/share/man/')):
                    continue
                if path.is_symlink():
                    resolved = path.resolve()
                    if not resolved.is_relative_to(package) or not resolved.exists():
                        continue
                    if resolved.is_file():
                        with resolved.open('rb') as source:
                            elf = source.read(4) == b'\x7fELF'
                        if elf and resolved not in wanted:
                            continue
                    copy_entry(path, payload / relative, package)
                elif path.is_file():
                    with path.open('rb') as source:
                        elf = source.read(4) == b'\x7fELF'
                    if elf and path.resolve() not in wanted:
                        continue
                    mode = path.stat().st_mode
                    if mode & (stat.S_ISUID | stat.S_ISGID):
                        if elf:
                            raise ValueError('Selected desktop ELF unexpectedly requires set-ID metadata')
                        continue
                    if not elf and mode & 0o111:
                        continue
                    copy_entry(path, payload / relative, package)
                    if elf and digest(path) != digest(payload / relative):
                        raise ValueError('Desktop assembly changed an upstream executable')
        source = ROOT / item['notice']
        if digest(source) != item['noticeSha256']:
            raise ValueError('Desktop copyright differs from its reviewed bytes')
        shutil.copy2(source, notices / (item['package'] + '.copyright'))
    for path in (ROOT / 'notices/desktop-full').iterdir():
        if not path.name.endswith('.copyright'):
            shutil.copy2(path, notices / path.name)
    for name in ['desktop-full.json', 'desktop-full.lock.json']:
        shutil.copy2(ROOT / name, notices / name)
    shutil.copy2(REPO / 'LICENSE.md', notices / 'pryx-LICENSE.md')
    binaries = output / 'bin'
    binaries.mkdir()
    libraries = ':'.join('$component/' + str(path.relative_to(output)) for path in library_paths(output))
    for name, relative in config['programs'].items():
        launcher = binaries / name
        launcher.write_text(f'''#!/bin/sh
# SPDX-License-Identifier: FSL-1.1-ALv2
set -eu
component=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd -P)
export LD_LIBRARY_PATH="{libraries}"
exec "$component/payload/{relative}" "$@"
''')
        launcher.chmod(0o755)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cache', type=Path)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--report', type=Path, required=True)
    parser.add_argument('--smoke-only', action='store_true')
    options = parser.parse_args()
    root = options.output.resolve()
    if not options.smoke_only:
        if options.cache is None:
            parser.error('Assembly requires the immutable input cache')
        assemble(options.cache, root)
    report = audit(root)
    observed = os.confstr('CS_GNU_LIBC_VERSION')
    if not observed or tuple(map(int, observed.split()[-1].split('.'))) < tuple(map(int, report['hostGlibcMinimum'].split('.'))):
        raise ValueError('Desktop controls require a sufficiently new host glibc')
    report['observedHostGlibc'] = observed
    for name, flag in [('Xvfb', '-help'), ('xdotool', '--version'), ('scrot', '--version'), ('x11vnc', '-version'), ('sshd', '-V'), ('ssh', '-V'), ('chromium', '--version')]:
        result = subprocess.run([str(root / 'bin' / name), flag], timeout=10, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        if result.returncode:
            raise ValueError(f'Desktop loader control failed for {name}: {result.stdout[-8192:]}')
    report['relocatedProgramControls'] = ['Xvfb', 'xdotool', 'scrot', 'x11vnc', 'sshd', 'ssh', 'chromium']
    options.report.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({k: v for k, v in report.items() if k != 'files'}))


if __name__ == '__main__':
    main()
