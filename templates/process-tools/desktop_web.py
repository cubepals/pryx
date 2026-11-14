# SPDX-License-Identifier: FSL-1.1-ALv2
"""Stage the already reviewed noVNC and websockify programs without installing extras.

Retain their complete source distributions, notices and immutable input metadata. The
native adapter uses only websockify's Unix socket and token-file functionality.
"""
import json
from pathlib import Path
import shutil
import tarfile
import zipfile
from archive_inputs import ROOT, acquire, unpack


def inputs():
    return json.loads((ROOT / 'desktop-web.lock.json').read_text())


def assemble(cache, component):
    web = component / 'web'
    web.mkdir()
    for item in inputs():
        archive = acquire(item, cache)
        if item['name'] == 'websockify-wheel':
            with zipfile.ZipFile(archive) as source:
                for name in source.namelist():
                    relative = Path(name)
                    if relative.is_absolute() or '..' in relative.parts:
                        raise ValueError('Unsafe websockify archive path')
                source.extractall(web / 'python')
        elif item['name'] == 'novnc':
            unpack(archive, web)
            (web / 'noVNC-1.7.0').rename(web / 'novnc')
        else:
            # Sources are data and stay available for redistribution, never executed.
            shutil.copy2(archive, component / 'notices/websockify-source.tar.gz')
            with tarfile.open(archive) as source:
                notice = source.extractfile('websockify-0.13.0/COPYING').read()
            (component / 'notices/websockify-COPYING').write_bytes(notice)
    shutil.copy2(ROOT / 'desktop-web.lock.json', component / 'notices')
    shutil.copytree(ROOT / 'desktop', component / 'adapter')


def sources(cache, output):
    output.mkdir()
    for item in inputs():
        source = acquire(item, cache)
        shutil.copy2(source, output / source.name)
    shutil.copy2(ROOT / 'desktop-web.lock.json', output)
