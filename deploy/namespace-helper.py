#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Stage the namespaced process mode's helper beside an agent executable.

It is the reviewed external bubblewrap and its one library, libcap, from the digest-pinned
conda-forge archives the process tool bundle already reviews (templates/process-tools/
helpers.lock.json and libraries.lock.json), with their complete licence texts; on aarch64, where
there is no process tool bundle, the same releases' aarch64 builds (namespaces-aarch64.lock.json).
The layout keeps
bubblewrap's own RPATH ($ORIGIN/../lib): `namespaces/bin/bwrap`, `namespaces/lib/libcap.so.2`.
No package hook runs and no binary is modified; it is never setuid.

    python3 -I deploy/namespace-helper.py <agent-directory> [--cache <directory>]
"""
import argparse
import io
import json
import platform
import shutil
import subprocess
import sys
import tarfile
import tempfile
import zipfile
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(REPO / "templates" / "process-tools"))
from archive_inputs import acquire, digest  # noqa: E402


# Each machine's (lock, package) pairs for bubblewrap and libcap, as `uname -m` names the machine.
INPUTS = {
    "x86_64": [("helpers.lock.json", "bubblewrap"), ("libraries.lock.json", "libcap")],
    "aarch64": [("namespaces-aarch64.lock.json", "bubblewrap"), ("namespaces-aarch64.lock.json", "libcap")],
}


def pinned(lock, name):
    for item in json.loads((REPO / "templates" / "process-tools" / lock).read_text()):
        if item["name"] == name:
            return item
    raise SystemExit(f"{lock} pins no {name}")


def members(archive, kind):
    """The `pkg` or `info` tarball of a .conda archive, decoded by the zstd program."""
    with zipfile.ZipFile(archive) as conda:
        names = [n for n in conda.namelist() if n.startswith(f"{kind}-") and n.endswith(".tar.zst")]
        if len(names) != 1:
            raise SystemExit(f"{archive.name} has no single {kind} tarball")
        encoded = conda.read(names[0])
    decoded = subprocess.run(["zstd", "-dc"], input=encoded, capture_output=True, check=True).stdout
    return tarfile.open(fileobj=io.BytesIO(decoded))


def take(item, cache, staging, files):
    archive = acquire(item, cache)
    unpacked = staging / item["name"]
    with members(archive, "pkg") as package:
        package.extractall(unpacked, filter="data")
    with members(archive, "info") as info:
        info.extractall(unpacked / "info-archive", filter="data")
    for notice in item["notices"]:
        path = unpacked / "info-archive" / notice["path"]
        if digest(path) != notice["sha256"]:
            raise SystemExit(f"{item['name']}: notice {notice['path']} differs from its review")
    return {name: unpacked / name for name in files}, [unpacked / "info-archive" / n["path"] for n in item["notices"]]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("agent_directory", type=Path)
    parser.add_argument("--cache", type=Path, default=Path(tempfile.gettempdir()) / "pryx-namespace-helper-cache")
    options = parser.parse_args()
    target = options.agent_directory / "namespaces"
    with tempfile.TemporaryDirectory() as temporary:
        staging = Path(temporary)
        machine = platform.machine()
        if machine not in INPUTS:
            raise SystemExit(f"No pinned namespace helper for {machine}")
        (helper_lock, helper_name), (library_lock, library_name) = INPUTS[machine]
        helper, helper_notices = take(pinned(helper_lock, helper_name), options.cache, staging, ["bin/bwrap"])
        library, library_notices = take(
            pinned(library_lock, library_name), options.cache, staging, ["lib/libcap.so.2.78"]
        )
        built = staging / "namespaces"
        (built / "bin").mkdir(parents=True)
        (built / "lib").mkdir()
        shutil.copyfile(helper["bin/bwrap"], built / "bin" / "bwrap")
        (built / "bin" / "bwrap").chmod(0o755)
        shutil.copyfile(library["lib/libcap.so.2.78"], built / "lib" / "libcap.so.2.78")
        (built / "lib" / "libcap.so.2.78").chmod(0o644)
        (built / "lib" / "libcap.so.2").symlink_to("libcap.so.2.78")
        for name, notices in [("bubblewrap", helper_notices), ("libcap", library_notices)]:
            (built / "licenses" / name).mkdir(parents=True)
            for notice in notices:
                shutil.copyfile(notice, built / "licenses" / name / notice.name)
        for directory in [built, built / "bin", built / "lib", built / "licenses"]:
            directory.chmod(0o755)
        options.agent_directory.mkdir(parents=True, exist_ok=True)
        if target.exists():
            shutil.rmtree(target)
        shutil.move(built, target)
    print(f"{target / 'bin' / 'bwrap'}")


if __name__ == "__main__":
    main()
