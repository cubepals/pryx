# SPDX-License-Identifier: FSL-1.1-ALv2
"""Build-time immutable archive retrieval and payload copying.

These operations are independent of runtime assembly and execute no package hooks. Runtime
acquisition has its own protected worker and does not import these build-time routines.
Retrieval waits out an archive's brief outage; the pinned digest alone decides whether the
bytes are accepted.
"""
import datetime
import hashlib
import http.client
from http import HTTPStatus
import os
from pathlib import Path
import shutil
import ssl
import sys
import tarfile
import time
import urllib.error
import urllib.request

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parent.parent

# A failed download is tried again, with doubling waits, until this long after its first try:
# long enough to wait out an archive's brief outage, short enough to end a job that can't.
RETRY_BUDGET = datetime.timedelta(minutes=3).total_seconds()
FIRST_RETRY_WAIT = datetime.timedelta(seconds=5).total_seconds()
LONGEST_RETRY_WAIT = datetime.timedelta(minutes=1).total_seconds()
# RFC 9110 section 15.6: 5xx is the server-error class, including codes Python's HTTPStatus
# doesn't name, such as a CDN's own 52x.
SERVER_ERRORS = range(500, 600)
NETWORK_ERRORS = (urllib.error.URLError, ConnectionError, TimeoutError, ssl.SSLError,
                  http.client.HTTPException)

# GNU libc's runtime sonames belong to the host, including the ELF interpreter.
GLIBC_SONAMES = frozenset({
    'libc.so.6', 'libm.so.6', 'libpthread.so.0', 'libdl.so.2', 'librt.so.1',
    'libresolv.so.2', 'libutil.so.1', 'libanl.so.1', 'ld-linux-x86-64.so.2',
})


def digest(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def transient(error):
    """Whether trying again could help: a refusal other than 5xx or 429 means a wrong pin."""
    if isinstance(error, urllib.error.HTTPError):
        return error.code in SERVER_ERRORS or error.code == HTTPStatus.TOO_MANY_REQUESTS
    return True


def download(url, destination):
    """Write url's bytes to destination, trying again within the budget while failures pass."""
    deadline = time.monotonic() + RETRY_BUDGET
    wait = FIRST_RETRY_WAIT
    while True:
        try:
            with urllib.request.urlopen(url, timeout=180) as response:
                with destination.open("wb") as output:
                    shutil.copyfileobj(response, output)
            return
        except NETWORK_ERRORS as error:
            if isinstance(error, urllib.error.HTTPError):
                error.close()  # Its body is an error page, never read.
            print(f"Download failed: {url}: {error}", file=sys.stderr, flush=True)
            if not transient(error) or time.monotonic() + wait > deadline:
                raise
        print(f"Trying again in {wait:g} s", file=sys.stderr, flush=True)
        time.sleep(wait)
        wait = min(2 * wait, LONGEST_RETRY_WAIT)


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


