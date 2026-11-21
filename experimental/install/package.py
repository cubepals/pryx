#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Pack a native lab executable for the release signer, checking its ELF architecture."""

import argparse
from pathlib import Path
import zipfile

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("binary", type=Path)
parser.add_argument("output", type=Path)
parser.add_argument("--arch", required=True, choices=["x64", "arm64"])
args = parser.parse_args()
binary = args.binary.read_bytes()
machine = {"x64": 62, "arm64": 183}[args.arch]
if binary[:6] != b"\x7fELF\x02\x01" or int.from_bytes(binary[18:20], "little") != machine:
    parser.error("expected a little-endian 64-bit ELF for the specified architecture")
with zipfile.ZipFile(args.output, "w") as archive:
    for name, data in [("__main__.py", Path(__file__).with_name("lab.py").read_bytes()),
                       ("controller.mjs", Path(__file__).with_name("controller.mjs").read_bytes()), ("pryx-lab", binary)]:
        archive.writestr(zipfile.ZipInfo(name), data)
