# SPDX-License-Identifier: FSL-1.1-ALv2
"""Bounded trusted-service startup diagnostics with stored credentials removed.

Reads only the image's private startup logs and service journal, never workload data or env files.
"""
import json
import os
from pathlib import Path
import re
import stat
import subprocess
import sys

root = Path(sys.argv[1])
private = root / "private"
secrets = []
try:
    info = private.lstat()
    if not stat.S_ISDIR(info.st_mode) or info.st_uid != os.getuid() or info.st_mode & 0o077:
        raise ValueError("unsafe diagnostic directory")
    fd = os.open(private / "credentials.json", os.O_RDONLY | os.O_NOFOLLOW)
    with os.fdopen(fd) as file:
        values = json.load(file)
    for value in values.values():
        if isinstance(value, str):
            secrets.extend(line for line in value.splitlines() if len(line) > 8)
except (OSError, ValueError):
    pass


def redacted(text):
    for secret in sorted(secrets, key=len, reverse=True):
        text = text.replace(secret, "[redacted]")
    text = re.sub(r"(?:pryx_|bk1\.|pa1\.)[A-Za-z0-9_.-]+", "[redacted]", text)
    text = re.sub(r"[0-9a-fA-F]{32,}", "[redacted]", text)
    return text


journal = subprocess.run(
    ["journalctl", "-u", "pryx-image", "--no-pager", "-n", "100", "-o", "cat"],
    capture_output=True, text=True, timeout=15, check=False,
)
print("Image service journal:\n" + redacted(journal.stdout[-65536:]))
try:
    for path in sorted(private.glob("*.log")):
        fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
        with os.fdopen(fd) as file:
            info = os.fstat(file.fileno())
            if not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid() or info.st_mode & 0o077:
                continue
            file.seek(max(0, info.st_size - 65536))
            print(path.name + ":\n" + redacted(file.read()))
except (OSError, ValueError):
    print("Private startup diagnostics are unavailable")
