#!/usr/bin/env python3
"""Compare the finalized JIT bytes with the native assembler's same specialization."""
import hashlib
import platform
import subprocess
import sys
import tempfile
from pathlib import Path

root = Path(__file__).resolve().parent
images = Path(sys.argv[1])
source = root / "machine" / "src" / (platform.machine() + ".S")
with tempfile.TemporaryDirectory() as tmp:
    obj = Path(tmp) / "static.o"
    text = Path(tmp) / "section"
    subprocess.run(["cc", "-c", source, "-o", obj], check=True)
    subprocess.run(["objcopy", "--dump-section", f".text={text}", obj], check=True)
    symbols = {}
    for line in subprocess.check_output(["nm", "-S", "--format=posix", obj], text=True).splitlines():
        fields = line.split()
        if fields[0].startswith("pryx_static_"):
            symbols[fields[0]] = (int(fields[2], 16), int(fields[3], 16))
    section = text.read_bytes()
    print("plan,byte_identical,static_sha256,generated_sha256")
    for image in sorted(images.glob("*.bin")):
        width, unroll = image.stem.split("-")
        start, size = symbols[f"pryx_static_{width[1:]}_{unroll[1:]}"]
        static, generated = section[start:start+size], image.read_bytes()
        print(f"{image.stem},{int(static == generated)},{hashlib.sha256(static).hexdigest()},"
              f"{hashlib.sha256(generated).hexdigest()}")
