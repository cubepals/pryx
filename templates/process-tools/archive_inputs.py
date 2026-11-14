# SPDX-License-Identifier: FSL-1.1-ALv2
"""Build-time immutable archive retrieval and payload copying.

These operations are independent of runtime assembly and execute no package hooks. Runtime
acquisition has its own protected worker and does not import these build-time routines.
"""
import hashlib
import os
from pathlib import Path
import shutil
import tarfile
import time
import urllib.request

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parent.parent

# GNU libc's runtime sonames belong to the host, including the ELF interpreter.
GLIBC_SONAMES = frozenset({
    'libc.so.6', 'libm.so.6', 'libpthread.so.0', 'libdl.so.2', 'librt.so.1',
    'libresolv.so.2', 'libutil.so.1', 'libanl.so.1', 'ld-linux-x86-64.so.2',
})


def digest(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def acquire(item, cache):
    target = cache / item["sha256"] / Path(item["url"]).name
    target.parent.mkdir(parents=True, exist_ok=True)
    if target.exists():
        if digest(target) != item["sha256"]:
            raise ValueError(f"Cached digest mismatch: {item['name']}")
        return target
    for attempt in range(3):
        try:
            with urllib.request.urlopen(item["url"], timeout=180) as response:
                with target.with_suffix(".download").open("wb") as output:
                    shutil.copyfileobj(response, output)
            if digest(target.with_suffix(".download")) != item["sha256"]:
                raise ValueError(f"Downloaded digest mismatch: {item['name']}")
            target.with_suffix(".download").rename(target)
            return target
        except (OSError, TimeoutError):
            if attempt == 2:
                raise
            time.sleep(5 * (attempt + 1))


def unpack(archive, destination):
    destination.mkdir(parents=True, exist_ok=True)
    with tarfile.open(archive) as source:
        source.extractall(destination, filter="data")


def copy_entry(source, destination, boundary):
    destination.parent.mkdir(parents=True, exist_ok=True)
    if source.is_symlink():
        link = os.readlink(source)
        resolved = (source.parent / link).resolve()
        if Path(link).is_absolute() or not resolved.is_relative_to(boundary.resolve()):
            raise ValueError(f"Escaping package link: {source}")
        if destination.is_symlink() and os.readlink(destination) == link:
            return
        destination.symlink_to(link)
    elif destination.exists():
        if digest(source) != digest(destination):
            raise ValueError(f"Conflicting package payload: {destination}")
    else:
        shutil.copy2(source, destination)


