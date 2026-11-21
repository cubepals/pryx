#!/usr/bin/env python3
"""Run every integration test executable Cargo built under native Valgrind."""
import argparse
import json
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("manifest")
parser.add_argument("feature")
args = parser.parse_args()
build = subprocess.run(["cargo", "test", "--manifest-path", args.manifest, "--features", args.feature,
                        "--locked", "--no-run", "--message-format=json"], text=True,
                       stdout=subprocess.PIPE, check=True)
tests = [message["executable"] for line in build.stdout.splitlines() if
         (message := json.loads(line))["reason"] == "compiler-artifact"
         and "test" in message["target"]["kind"] and message.get("executable")]
if not tests:
    raise RuntimeError("no integration test binaries produced")
for binary in sorted(set(tests)):
    subprocess.run(["valgrind", "--tool=memcheck", "--error-exitcode=88", "--leak-check=full",
                    "--errors-for-leak-kinds=definite", binary], check=True)
