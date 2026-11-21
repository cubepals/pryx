#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Sign an existing lab executable with the release environment's Ed25519 key."""

import argparse
from pathlib import Path
import subprocess
import tempfile

from lab_release import statement

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("binary", type=Path)
parser.add_argument("signature", type=Path)
parser.add_argument("--key", type=Path, required=True)
parser.add_argument("--public-key", type=Path, help="verify the result with this pinned release public key")
parser.add_argument("--version", required=True)
parser.add_argument("--arch", choices=["x64", "arm64"], required=True)
args = parser.parse_args()
with tempfile.TemporaryDirectory(dir=args.signature.parent) as folder:
    message = Path(folder) / "statement"
    message.write_bytes(statement(args.binary, args.version, args.arch))
    output = Path(folder) / "signature"
    subprocess.run(["openssl", "pkeyutl", "-sign", "-rawin", "-inkey", args.key, "-in", message,
                    "-out", output], check=True)
    if args.public_key:
        subprocess.run(["openssl", "pkeyutl", "-verify", "-pubin", "-inkey", args.public_key,
                        "-rawin", "-in", message, "-sigfile", output], check=True)
    output.replace(args.signature)
