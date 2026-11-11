#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Assemble unchanged upstream tools and verified library payloads, without prefix rewriting.

This build-time program produces a directory and its inventory; runtime acquisition verifies
the finished archive separately. It never runs package hooks or includes a private libc.
"""

import argparse
from contextlib import ExitStack
import hashlib
import json
import os
import re
from pathlib import Path
import shutil
import stat
import subprocess
import tarfile
import tempfile
import time
import urllib.request
import zipfile

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parent.parent


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


def libraries(items, cache, output, extractor):
    forbidden = {"libc.so.6", "libm.so.6", "libpthread.so.0", "libdl.so.2", "librt.so.1", "libresolv.so.2"}
    for item in items:
        archive = acquire(item, cache)
        # Extract verified bytes afresh; a mutable extracted cache is not integrity evidence.
        with tempfile.TemporaryDirectory(dir=cache) as staging:
            package = Path(staging) / "payload"
            subprocess.run([str(extractor), "package", "extract", "--no-rc", str(archive), str(package)], check=True)
            library_payload(item, package, output, forbidden)


def library_payload(item, package, output, forbidden):
    notice_root = output / "notices" / "libraries" / item["name"]
    for notice in item["notices"]:
        source = package / notice["path"]
        if digest(source) != notice["sha256"]:
            raise ValueError(f"Package notice mismatch: {source}")
        copy_entry(source, notice_root / notice["path"], package)
    for source in package.rglob("*"):
        relative = source.relative_to(package)
        if source.is_dir():
            continue
        library = relative.parts[0] == "lib" and ".so" in source.name
        font = relative.parts[:2] == ("share", "fonts") or relative.parts[0] == "fonts"
        if not library and not font:
            continue
        if source.name in forbidden or source.name.startswith("ld-linux"):
            raise ValueError(f"A private glibc is forbidden: {source}")
        target = Path("share") / relative if relative.parts[0] == "fonts" else relative
        copy_entry(source, output / "libraries" / target, package)


def helpers(items, cache, output, extractor):
    for item in items:
        archive = acquire(item, cache)
        with tempfile.TemporaryDirectory(dir=cache) as staging:
            package = Path(staging) / "payload"
            subprocess.run([str(extractor), "package", "extract", "--no-rc", str(archive), str(package)], check=True)
            library_payload(item, package, output, set())
            for source in (package / "bin").iterdir():
                copy_entry(source, output / "helpers" / source.name, package)
                (output / "helpers" / source.name).chmod(0o755)
            shutil.copytree(package / "info/recipe", output / "notices/libraries" / item["name"] / "recipe")


def builder(cache, provided, lifetime):
    review = json.loads((REPO / "docs/checks/measurements/k23/micromamba-builder.json").read_text())
    archive = acquire(review, cache)
    with tarfile.open(archive) as source:
        for notice in review["notices"]:
            if hashlib.sha256(source.extractfile(notice["path"]).read()).hexdigest() != notice["sha256"]:
                raise ValueError("Builder licence notice mismatch")
        binary_digest = hashlib.sha256(source.extractfile("bin/micromamba").read()).hexdigest()
    if provided is None:
        staging = Path(lifetime.enter_context(tempfile.TemporaryDirectory(dir=cache)))
        unpack(archive, staging)
        provided = staging / "bin/micromamba"
    if digest(provided) != binary_digest:
        raise ValueError("Builder executable does not match its reviewed archive")
    return provided


def python_notices(archive, output):
    destination = output / "notices" / "python"
    destination.mkdir(parents=True)
    with subprocess.Popen(["zstd", "-dc", str(archive)], stdout=subprocess.PIPE) as decoder:
        with tarfile.open(fileobj=decoder.stdout, mode="r|") as source:
            for member in source:
                path = Path(member.name)
                if not member.isfile() or not (member.name == "python/PYTHON.json" or member.name.startswith("python/licenses/")):
                    continue
                if ".." in path.parts or path.is_absolute():
                    raise ValueError("Invalid Python notice path")
                target = destination / path.relative_to("python")
                target.parent.mkdir(parents=True, exist_ok=True)
                with source.extractfile(member) as contents, target.open("wb") as file:
                    shutil.copyfileobj(contents, file)
        if decoder.wait() != 0:
            raise ValueError("Python notice decompression failed")


def runtimes(items, cache, output):
    for item in items:
        archive = acquire(item, cache)
        if item["name"] == "python-standalone":
            unpack(archive, output)
        elif item["name"] == "node":
            with tempfile.TemporaryDirectory(dir=cache) as staging:
                unpack(archive, Path(staging))
                (Path(staging) / f"node-v{item['version']}-linux-x64").rename(output / "node")
        elif item["name"] == "headless-shell":
            with zipfile.ZipFile(archive) as source:
                for entry in source.infolist():
                    relative = Path(entry.filename)
                    if relative.is_absolute() or ".." in relative.parts:
                        raise ValueError("Invalid browser archive path")
                    if entry.is_dir():
                        continue
                    mode = entry.external_attr >> 16
                    if stat.S_ISLNK(mode):
                        raise ValueError("Unexpected browser archive symlink")
                    target = output / "browser" / relative
                    target.parent.mkdir(parents=True, exist_ok=True)
                    with source.open(entry) as content, target.open("wb") as file:
                        shutil.copyfileobj(content, file)
                    target.chmod(mode & 0o777)
        elif item["name"] == "python-standalone-notices":
            python_notices(archive, output)


def node_packages(source, destination, output, environment, select_addons=False):
    with tempfile.TemporaryDirectory() as directory:
        staging = Path(directory)
        for name in ["package.json", "package-lock.json"]:
            shutil.copy2(source / name, staging / name)
        subprocess.run([
            str(output / "bin/node"), str(output / "node/lib/node_modules/npm/bin/npm-cli.js"),
            "ci", "--ignore-scripts", "--omit=dev", "--no-audit", "--no-fund",
        ], env=environment, cwd=staging, check=True)
        selected = set()
        if select_addons:
            script = "require('zeromq');require('@napi-rs/canvas');console.log(JSON.stringify(process.report.getReport().sharedObjects.filter(p=>p.endsWith('.node'))))"
            selected = set(json.loads(subprocess.check_output([
                str(output / "bin/node"), "-e", script,
            ], cwd=staging, env=environment)))

        def omit(parent, names):
            # Retain only the upstream addons actually selected by this pinned Node build.
            return [name for name in names if select_addons and name.endswith(".node")
                    and str(Path(parent) / name) not in selected]

        shutil.copytree(staging, destination, ignore=omit, symlinks=True)
    for source_file in source.iterdir():
        if source_file.suffix in {".cjs", ".mjs"}:
            shutil.copy2(source_file, destination / source_file.name)


def kernels(output):
    environment = os.environ.copy()
    environment["PATH"] = f"{output / 'bin'}:{environment.get('PATH', '')}"
    environment["PIP_DISABLE_PIP_VERSION_CHECK"] = "1"
    subprocess.run([
        str(output / "bin/python3"), "-m", "pip", "install", "--only-binary=:all:",
        "--require-hashes", "-r", str(ROOT.parent / "python-interpreter/requirements.txt"),
    ], env=environment, check=True)
    target = output / "kernels/javascript"
    node_packages(ROOT.parent / "node-interpreter", target, output, environment, select_addons=True)
    shutil.copytree(ROOT.parent / "node-interpreter/notices", target / "notices")
    node_packages(ROOT / "browser", output / "browser-client", output, environment)
    shutil.copytree(ROOT.parent / "python-interpreter/ipython", output / "kernels/python/ipython")


def configuration(output):
    shutil.copytree(ROOT / "launchers", output / "bin")
    fonts = output / "etc/fonts/fonts.conf"
    fonts.parent.mkdir(parents=True)
    # Fontconfig resolves relative dirs from this file; no embedded build prefix or host font dependency.
    fonts.write_text('<?xml version="1.0"?><!DOCTYPE fontconfig SYSTEM "fonts.dtd">\n'
                     '<fontconfig><dir prefix="relative">../../libraries/share/fonts</dir>'
                     '<cachedir prefix="xdg">fontconfig</cachedir></fontconfig>\n')
    shutil.copytree(ROOT / "notices", output / "notices/corrections")
    for name in ["artifacts.json", "libraries.lock.json", "helpers.lock.json", "compatibility.json"]:
        shutil.copy2(ROOT / name, output / "notices" / name)
    shutil.copy2(REPO / "docs/checks/measurements/k23/python-standalone-review.json", output / "notices/python-review.json")


def inventory(output):
    entries = []
    minimum = tuple(map(int, json.loads((ROOT / "compatibility.json").read_text())["hostGlibcMinimum"].split('.')))
    required = (0, 0)
    for path in sorted(output.rglob("*")):
        if path.is_symlink():
            if not path.resolve().is_relative_to(output):
                raise ValueError(f"Bundle symlink escapes: {path}")
            entries.append({"path": str(path.relative_to(output)), "link": os.readlink(path)})
        elif path.is_file():
            with path.open("rb") as file:
                elf = file.read(4) == b"\x7fELF"
            if elf:
                symbols = subprocess.check_output(["readelf", "--version-info", "--wide", str(path)], stderr=subprocess.DEVNULL).decode()
                versions = [tuple(map(int, value.split('.'))) for value in re.findall(r"GLIBC_([0-9]+\.[0-9]+(?:\.[0-9]+)?)", symbols)]
                required = max([required, *versions])
                if required > minimum:
                    raise ValueError(f"Bundle ELF exceeds host glibc minimum: {path}")
            entries.append({"path": str(path.relative_to(output)), "sha256": digest(path)})
    (output / "manifest.json").write_text(json.dumps({
        "format": 1, **json.loads((ROOT / "compatibility.json").read_text()),
        "maximumRequiredGlibc": '.'.join(map(str, required)), "files": entries,
    }, indent=2) + "\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cache", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--micromamba", type=Path)
    options = parser.parse_args()
    output = options.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    options.cache.mkdir(parents=True, exist_ok=True)
    with ExitStack() as lifetime:
        extractor = builder(options.cache, options.micromamba, lifetime)
        runtimes(json.loads((ROOT / "artifacts.json").read_text()), options.cache, output)
        libraries(json.loads((ROOT / "libraries.lock.json").read_text()), options.cache, output, extractor)
        helpers(json.loads((ROOT / "helpers.lock.json").read_text()), options.cache, output, extractor)
        configuration(output)
        kernels(output)
        inventory(output)


if __name__ == "__main__":
    main()
