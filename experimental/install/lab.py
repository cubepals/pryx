# SPDX-License-Identifier: FSL-1.1-ALv2
"""Install and run a release-authenticated lab archive for one user.

The stock CLI authenticates a private copy of this archive before every invocation. Activation
publishes the archive, its detached signature and its selection with one atomic link replacement.
This program does not change the daemon, host configuration, or stock CLI.
"""

from contextlib import contextmanager
import fcntl
import hashlib
import json
import os
import re
from pathlib import Path
import shutil
import stat
import subprocess
import sys
import tempfile
import uuid
import zipfile

FEATURE = "self-check"


def protected(path, kind):
    info = path.lstat()
    if info.st_uid != os.getuid() or info.st_mode & 0o022 or not kind(info.st_mode):
        raise ValueError(f"unexpected ownership, permissions or type at {path}")
    return path


def storage(create=False):
    home = protected(Path.home(), stat.S_ISDIR)
    root = home / ".pryx-experimental"
    if create:
        root.mkdir(mode=0o700, exist_ok=True)
    return protected(root, stat.S_ISDIR)


def selection(args):
    if args == []:
        return ""
    if args == ["--feature", FEATURE]:
        return FEATURE
    raise ValueError(f"supported selection: --feature {FEATURE}")


def digest(path):
    with path.open("rb") as stream:
        result = hashlib.sha256()
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            result.update(chunk)
        return result.digest()


def generation_name(archive, signature, selected):
    fingerprint = hashlib.sha256(digest(archive) + digest(signature) + hashlib.sha256(selected).digest())
    return "build-" + fingerprint.hexdigest()


def recorded_selection(generation, archive, signature):
    record = protected(generation / "selection", stat.S_ISREG)
    with record.open("rb") as stream:
        data = stream.read(257)
    if len(data) > 256:
        raise ValueError("invalid installed feature selection")
    name = generation_name(archive, signature, data)
    if not re.fullmatch(name + r"(?:-[a-f0-9]{32})?", generation.name):
        raise ValueError("installed feature selection failed its content check")
    feature = data.decode("ascii")
    return selection(["--feature", feature] if feature else [])


@contextmanager
def activation_lock(root):
    # Readers pin an immutable directory directly. Only writers coordinate pointer changes.
    directory = os.open(root, os.O_DIRECTORY | os.O_NOFOLLOW | os.O_RDONLY)
    try:
        fcntl.flock(directory, fcntl.LOCK_EX)
        yield
    finally:
        os.close(directory)


def matches(path, name, archive_size, feature):
    if not re.fullmatch(name + r"(?:-[a-f0-9]{32})?", path.name):
        return False
    try:
        generation = protected(path, stat.S_ISDIR)
        archive = protected(generation / "lab.pyz", stat.S_ISREG)
        signature = protected(generation / "signature", stat.S_ISREG)
        return (archive.stat().st_size == archive_size and signature.stat().st_size == 64
                and recorded_selection(generation, archive, signature) == feature)
    except (OSError, ValueError):
        return False


def activate(archive, signature, feature):
    root = storage(create=True)
    selected = feature.encode("ascii")
    name = generation_name(archive, signature, selected)
    size = archive.stat().st_size
    with activation_lock(root), tempfile.TemporaryDirectory(dir=root) as scratch:
        scratch = Path(scratch)
        link = root / "active"
        try:
            active = os.readlink(link)
        except FileNotFoundError:
            active = ""
        if active and "/" not in active and matches(root / active, name, size, feature):
            return
        canonical = root / name
        # An interrupted install may already have prepared a valid destination without switching
        # the pointer. Reuse that destination on retry, including previous same-content repairs.
        candidates = [canonical, *sorted(root.glob(f"{name}-*"))]
        target = next((p for p in candidates if matches(p, name, size, feature)), None)
        if target is None:
            new = scratch / "generation"
            new.mkdir(mode=0o700)
            shutil.copyfile(archive, new / "lab.pyz")
            shutil.copyfile(signature, new / "signature")
            (new / "selection").write_bytes(selected)
            target = root / f"{name}-{uuid.uuid4().hex}" if os.path.lexists(canonical) else canonical
            new.rename(target)
        pointer = scratch / "active"
        pointer.symlink_to(target.name)
        pointer.replace(link)


def main():
    if sys.version_info < (3, 8):
        raise ValueError("the lab requires Python 3.8 or later")
    command, *args = sys.argv[1:]
    archive = Path(sys.argv[0])
    if command == "controller" and not args:
        with zipfile.ZipFile(archive) as payload:
            sys.stdout.buffer.write(payload.read("controller.mjs"))
    elif command == "validate":
        selection(args)
    elif command == "activate" and args:
        activate(archive, Path(args[0]), selection(args[1:]))
    elif command == "capabilities" and len(args) == 2:
        try:
            generation = protected(Path(args[0]), stat.S_ISDIR)
            selected = recorded_selection(generation, archive, Path(args[1]))
            state = {"on": bool(selected),
                     "reason": "explicitly selected" if selected else "feature was not selected"}
        except (OSError, ValueError) as error:
            state = {"on": False, "reason": str(error),
                     "action": "Reinstall with pryx experimental install --experimental and the desired --feature"}
        print(json.dumps({FEATURE: state}))
    elif command == "run" and len(args) == 3:
        generation = protected(Path(args[0]), stat.S_ISDIR)
        selected = recorded_selection(generation, archive, Path(args[1]))
        if args[2] != FEATURE or selected != FEATURE:
            raise ValueError(f"the requested experiment must be selected with --feature {FEATURE}")
        with tempfile.TemporaryDirectory(dir=storage()) as scratch, zipfile.ZipFile(archive) as payload:
            binary = Path(scratch) / "pryx-lab"
            binary.write_bytes(payload.read("pryx-lab"))
            binary.chmod(0o700)
            subprocess.run([binary, FEATURE], check=True)
    else:
        raise ValueError("unsupported lab command")


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        sys.exit(f"pryx experimental: {error}")
