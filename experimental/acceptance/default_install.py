# SPDX-License-Identifier: FSL-1.1-ALv2
"""Audit artifacts and requests from the existing native one-command installation."""
import argparse
import hashlib
import json
import re
import tarfile
from pathlib import Path
from urllib.parse import unquote, urlsplit


def lab_path(path):
    return any(part.startswith("pryx-lab") or part == "experimental" for part in path.split("/"))


def manifest(archive):
    members = {}
    with tarfile.open(archive, "r|gz") as source:
        for item in source:
            if item.name in members or lab_path(item.name):
                raise ValueError(f"duplicate or lab member in {archive.name}: {item.name}")
            if item.isdir():
                value = "directory"
            elif item.isfile():
                stream = source.extractfile(item)
                prefix = stream.read(4)
                digest = hashlib.sha256(prefix)
                for chunk in iter(lambda: stream.read(1024 * 1024), b""):
                    digest.update(chunk)
                # The authorized CLI change can alter this executable alone.
                native_cli = item.name in ("bin/pryx", "bin/pryx.real") and prefix == b"\x7fELF"
                value = "stock-cli" if native_cli else digest.hexdigest()
            else:
                raise ValueError(f"unexpected archive member type {item.name}")
            members[item.name] = (item.mode, item.uid, item.gid, value)
    if not members:
        raise ValueError(f"empty archive {archive.name}")
    return members


def audit(directory, base, log, arch):
    requests = []
    for line in log.read_text().splitlines():
        match = re.search(r'"(?:GET|HEAD) ([^ ]+) HTTP/[0-9.]+" (\d+)', line)
        if match:
            requests.append((unquote(urlsplit(match[1]).path).lstrip("/"), int(match[2])))
    successful = {name for name, status in requests if 200 <= status < 300}
    required = {"install.sh", "SHA256SUMS", f"pryx-bootstrap-linux-{arch}.tar.gz",
                f"pryx-bundle-linux-{arch}.tar.gz"}
    if missing := required - successful:
        raise ValueError(f"missing successful installer requests: {sorted(missing)}")
    if any(lab_path(name) for name, _ in requests):
        raise ValueError("default installation requested a lab asset")
    if any(lab_path(path.name) for path in directory.iterdir()):
        raise ValueError("default release contains a lab asset")
    entries = {}
    for archive in sorted(name for name in required if name.endswith(".tar.gz")):
        before, after = manifest(base / archive), manifest(directory / archive)
        changed = [key for key in sorted(before.keys() | after.keys()) if before.get(key) != after.get(key)]
        if changed:
            raise ValueError(f"{archive} differs from the base in {changed[:20]}")
        entries[archive] = len(after)
    return {"arch": arch, "successful_requests": sorted(successful),
            "archive_members_checked": entries, "lab_requests": 0}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("artifacts", type=Path)
    parser.add_argument("base_artifacts", type=Path)
    parser.add_argument("access_log", type=Path)
    parser.add_argument("arch", choices=["x64", "arm64"])
    args = parser.parse_args()
    print(json.dumps(audit(args.artifacts, args.base_artifacts, args.access_log, args.arch), sort_keys=True))
