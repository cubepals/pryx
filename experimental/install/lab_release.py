# SPDX-License-Identifier: FSL-1.1-ALv2
"""Construct the lab archive's Ed25519 signing statement."""

import hashlib
import re


def statement(archive, version, arch):
    if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", version) or arch not in ("x64", "arm64"):
        raise ValueError("invalid lab release identity")
    with archive.open("rb") as stream:
        digest = hashlib.file_digest(stream, "sha256").hexdigest()
    return f"pryx experimental archive\n{version}\n{arch}\n{digest}\n".encode("ascii")
