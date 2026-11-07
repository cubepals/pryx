#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Stage the pinned agentOS distribution after integrity verification, without executing it.

Only immutable build inputs are accepted. This is not a runtime package installer or a shell
bootstrap: the native client and software registry retain their own runtime validation.
"""

import argparse
import base64
import hashlib
import json
from pathlib import Path, PurePosixPath
import shutil
import tarfile
import tempfile
import urllib.request


# Local build admission bounds, independent of upstream protocol or VM resource limits.
MAX_ARCHIVE_BYTES = 256 * 1024 * 1024
MAX_EXTRACTED_BYTES = 1024 * 1024 * 1024
CHUNK_BYTES = 1024 * 1024
REPOSITORY = Path(__file__).resolve().parent.parent


def archive(url, expected, directory):
    digest = hashlib.sha512()
    destination = directory / "download.tgz"
    count = 0
    with urllib.request.urlopen(url, timeout=60) as response, destination.open("xb") as output:
        if not response.url.startswith("https://"):
            raise ValueError("agentOS archive redirect must retain HTTPS")
        while chunk := response.read(CHUNK_BYTES):
            count += len(chunk)
            if count > MAX_ARCHIVE_BYTES:
                raise ValueError("agentOS archive exceeds the build admission bound")
            digest.update(chunk)
            output.write(chunk)
    actual = "sha512-" + base64.b64encode(digest.digest()).decode("ascii")
    if actual != expected:
        raise ValueError("agentOS archive integrity mismatch")
    return destination


def unpack(source, destination):
    total = 0
    with tarfile.open(source, "r:gz") as package:
        for member in package:
            path = PurePosixPath(member.name)
            if path.is_absolute() or ".." in path.parts or path.parts[:1] != ("package",):
                raise ValueError("agentOS archive contains an unsafe path")
            if member.isdir():
                continue
            if not member.isfile():
                raise ValueError("agentOS archive contains a link or special file")
            total += member.size
            if total > MAX_EXTRACTED_BYTES:
                raise ValueError("agentOS extracted package exceeds the build admission bound")
            target = destination.joinpath(*path.parts[1:])
            target.parent.mkdir(parents=True, exist_ok=True)
            with package.extractfile(member) as content, target.open("xb") as output:
                shutil.copyfileobj(content, output, CHUNK_BYTES)
            target.chmod(0o644)


def software(lock, item, staged):
    target = staged / "agentos-core/dist/optional-software" / (item["name"] + ".aospkg")
    target.parent.mkdir(parents=True, exist_ok=True)
    url = lock["softwareRegistry"] + lock["version"] + "/software/" + item["path"]
    digest = hashlib.sha256()
    count = 0
    request=urllib.request.Request(url,headers={"User-Agent":"pryx-distribution-fetch"})
    with urllib.request.urlopen(request, timeout=60) as response, target.open("xb") as output:
        if not response.url.startswith("https://"):
            raise ValueError("agentOS software redirect must retain HTTPS")
        while chunk := response.read(CHUNK_BYTES):
            count += len(chunk)
            if count > item["size"] or count > MAX_ARCHIVE_BYTES:
                raise ValueError("agentOS software exceeds its pinned size")
            digest.update(chunk)
            output.write(chunk)
    if count != item["size"] or "sha256:" + digest.hexdigest() != item["digest"]:
        raise ValueError("agentOS optional software checksum mismatch")
    target.chmod(0o644)


def stage(output):
    lock = json.loads((REPOSITORY / "deploy/agentos.json").read_text())
    if output.exists() or output.is_symlink():
        raise ValueError("agentOS output must not exist; stage a new build directory")
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".agentos-", dir=output.parent) as temporary:
        temporary = Path(temporary)
        staged = temporary / "distribution"
        staged.mkdir()
        for item in lock["artifacts"]:
            name = item["package"]
            url = f"https://registry.npmjs.org/@rivet-dev/{name}/-/{name}-{lock['version']}.tgz"
            with tempfile.TemporaryDirectory(dir=temporary) as download:
                source = archive(url, item["integrity"], Path(download))
                unpack(source, staged / name)
        binary = staged / "agentos-runtime-sidecar-linux-x64-gnu/agentos-native-sidecar"
        with binary.open("rb") as content:
            if hashlib.file_digest(content, "sha256").hexdigest() != lock["binarySha256"]:
                raise ValueError("agentOS sidecar binary checksum mismatch")
        binary.chmod(0o755)
        for item in lock.get("optionalPackages", []):
            software(lock, item, staged)
        licenses = staged / "licenses"
        licenses.mkdir()
        shutil.copyfile(REPOSITORY / "LICENSES/Apache-2.0.txt", licenses / "Apache-2.0.txt")
        shutil.copyfile(REPOSITORY / "deploy/agentos.NOTICE", staged / "NOTICE")
        shutil.copyfile(REPOSITORY / "deploy/agentos.json", staged / "distribution.json")
        staged.rename(output)
    print(f"Verified agentOS {lock['version']} distribution: {output}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    stage(parser.parse_args().output.absolute())
