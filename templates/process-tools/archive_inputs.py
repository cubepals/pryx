# SPDX-License-Identifier: FSL-1.1-ALv2
"""Build-time immutable archive retrieval and payload copying.

These operations are independent of runtime assembly and execute no package hooks. Runtime
acquisition has its own protected worker and does not import these build-time routines.
Retrieval waits out an archive's brief outage and tries the archive's mirrors (`mirrors.json`)
under the templates' one retry policy (`../archive_retry.py`); the pinned digest alone decides
whether the bytes are accepted.
"""
import hashlib
import json
import os
from pathlib import Path
import shutil
import sys
import tarfile

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parent.parent
sys.path.insert(0, str(ROOT.parent))
import archive_retry

MIRRORS = json.loads((ROOT / "mirrors.json").read_text())["archives"]

# GNU libc's runtime sonames belong to the host, including the ELF interpreter.
GLIBC_SONAMES = frozenset({
    'libc.so.6', 'libm.so.6', 'libpthread.so.0', 'libdl.so.2', 'librt.so.1',
    'libresolv.so.2', 'libutil.so.1', 'libanl.so.1', 'ld-linux-x86-64.so.2',
})


def digest(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def addresses(url):
    """The pinned address, then the same path on each mirror of the archive it starts with."""
    for archive in MIRRORS:
        if url.startswith(archive["prefix"]):
            path = url.removeprefix(archive["prefix"])
            return [url, *(mirror + path for mirror in archive["mirrors"])]
    return [url]


def download(url, destination):
    """Write the bytes of url, or of a mirror's copy, under the shared retry policy."""
    def save(response):
        with destination.open("wb") as output:
            shutil.copyfileobj(response, output)
    archive_retry.request(url, save, mirrors=addresses(url)[1:])


def acquire(item, cache):
    target = cache / item["sha256"] / Path(item["url"]).name
    target.parent.mkdir(parents=True, exist_ok=True)
    if target.exists():
        if digest(target) != item["sha256"]:
            raise ValueError(f"Cached digest mismatch: {item['name']}")
        return target
    partial = target.with_suffix(".download")
    download(item["url"], partial)
    if digest(partial) != item["sha256"]:
        raise ValueError(f"Downloaded digest mismatch: {item['name']}")
    partial.rename(target)
    return target


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


