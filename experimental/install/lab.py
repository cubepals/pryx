# SPDX-License-Identifier: FSL-1.1-ALv2
"""Install and run a release-authenticated lab archive for one user.

The stock CLI authenticates a private copy of this archive before every invocation. Activation
publishes the archive, its detached signature and its selection with one atomic link replacement.
This program does not change the daemon, host configuration, or stock CLI.
"""

import hashlib
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import sys
import tempfile
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


def activate(archive, signature, feature):
    root = storage(create=True)
    with tempfile.TemporaryDirectory(dir=root) as scratch:
        scratch = Path(scratch)
        new = scratch / "generation"
        new.mkdir(mode=0o700)
        shutil.copyfile(archive, new / "lab.pyz")
        shutil.copyfile(signature, new / "signature")
        (new / "selection").write_text(feature)
        identity = hashlib.sha256()
        for name in ("lab.pyz", "signature", "selection"):
            identity.update(digest(new / name))
        target = root / f"build-{identity.hexdigest()}"
        try:
            new.rename(target)
        except OSError:
            protected(target, stat.S_ISDIR)
            for item in new.iterdir():
                if digest(protected(target / item.name, stat.S_ISREG)) != digest(item):
                    raise ValueError("existing lab generation failed its content check")
        link = scratch / "active"
        link.symlink_to(target.name)
        link.replace(root / "active")


def main():
    if sys.version_info < (3, 8):
        raise ValueError("the lab requires Python 3.8 or later")
    command, *args = sys.argv[1:]
    archive = Path(sys.argv[0])
    if command == "validate":
        selection(args)
    elif command == "activate" and args:
        activate(archive, Path(args[0]), selection(args[1:]))
    elif command in ("capabilities", "run") and args:
        selected = selection(["--feature", args[0]] if args[0] else [])
        if command == "capabilities" and len(args) == 1:
            print(json.dumps({FEATURE: {"on": bool(selected),
                  "reason": "explicitly selected" if selected else "feature was not selected"}}))
        elif command == "run" and args == [FEATURE, FEATURE]:
            with tempfile.TemporaryDirectory(dir=storage()) as scratch, zipfile.ZipFile(archive) as payload:
                binary = Path(scratch) / "pryx-lab"
                binary.write_bytes(payload.read("pryx-lab"))
                binary.chmod(0o700)
                subprocess.run([binary, FEATURE], check=True)
        else:
            raise ValueError(f"the requested experiment must be selected with --feature {FEATURE}")
    else:
        raise ValueError("unsupported lab command")


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        sys.exit(f"pryx experimental: {error}")
