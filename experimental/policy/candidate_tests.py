#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Run the pinned stock process_network integration target with a candidate object on disposable CI."""

import argparse
import hashlib
import json
import mmap
import os
import platform
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import textwrap

STOCK = "35c28a630404d34fcc451528369432a54abba496"
HERE = Path(__file__).resolve().parent
CI_BLOB = "670ba549f040890e768f1969a51f2920c80b94f7"
AGENT = "/usr/local/lib/pryx-k31/pryx-agent"


def call(args, **options):
    return subprocess.run(args, check=True, **options)


def output(args, **options):
    return subprocess.check_output(args, text=True, **options).strip()


def sha256(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def setup_recipe(checkout):
    recipe = checkout / ".github/workflows/ci.yml"
    if output(["git", "hash-object", recipe]) != CI_BLOB:
        raise ValueError("The reviewed stock CI recipe changed")
    text = recipe.read_text()
    first = text.index("      - name: Destinations for the kernel network policy\n")
    last = text.index("      - name: Kernel network policy as root\n", first)
    return textwrap.dedent(text[first:last].split("        run: |\n", 1)[1])


def build(checkout, directory, env, experiment):
    embedded = checkout / "bpf/network.bpf.o"
    stock = selected = embedded
    if experiment:
        from candidate_image import build_images

        stock, selected = build_images(checkout, directory / "objects")
        if embedded.read_bytes() != stock.read_bytes() or stock.read_bytes() == selected.read_bytes():
            raise ValueError("The candidate must replace exactly the stock object")
        shutil.copyfile(selected, embedded)
        if output(["git", "diff", "--name-only"], cwd=checkout) != "bpf/network.bpf.o":
            raise ValueError("Staged candidate changed an additional tracked source")
    call(["cargo", "build", "--locked", "-p", "pryx-agent", "--bin", "pryx-agent"], cwd=checkout, env=env)
    command = ["cargo", "test", "--locked", "-p", "pryxd", "--test", "process_network", "--no-run",
               "--message-format=json-render-diagnostics"]
    records = [json.loads(line) for line in output(command, cwd=checkout, env=env).splitlines()]
    binaries = [Path(rec["executable"]) for rec in records if rec.get("executable")
                and rec["target"]["name"] == "process_network" and rec["profile"]["test"]]
    if len(binaries) != 1:
        raise ValueError("Expected exactly one unchanged process_network integration executable")
    binary = binaries[0]
    with binary.open("rb") as stream, mmap.mmap(stream.fileno(), 0, access=mmap.ACCESS_READ) as exe:
        if exe.find(selected.read_bytes()) < 0 or (experiment and exe.find(stock.read_bytes()) >= 0):
            raise ValueError("The test executable does not contain the selected policy object exclusively")
    tests = [line[:-6] for line in output([binary, "--ignored", "--list"], cwd=checkout).splitlines()
             if line.endswith(": test")]
    if len(tests) != 14 or not any(name.startswith("legacy::") for name in tests):
        raise ValueError("The unchanged root policy and legacy coverage were not discovered")
    return binary, tests, stock, selected


def run_suite(checkout, binary, tests, fixture, log, status):
    # The private directory supplied to the stock HTTP fixture contains no release credentials.
    # Its four PRYX_TEST_* addresses and its fixed agent installation come from stock CI verbatim.
    status["phase"] = "prepare_stock_destinations"
    fixture.mkdir(mode=0o700)
    outputs = fixture / "environment"
    env = os.environ | {"RUNNER_TEMP": str(fixture), "GITHUB_ENV": str(outputs),
                        "GITHUB_STEP_SUMMARY": str(fixture / "summary")}
    if any(line.split()[0] == "pryx-private" for line in output(["ip", "netns", "list"]).splitlines()):
        raise ValueError("The stock test namespace is already in use")
    if Path(AGENT).exists() or Path("/sys/class/net/pryx-host").exists():
        raise ValueError("The disposable runner already contains a stock test fixture")
    try:
        call(["bash", "-euo", "pipefail", "-c", setup_recipe(checkout)], cwd=checkout, env=env)
        fields = dict(line.split("=", 1) for line in outputs.read_text().splitlines())
        required = {"PRYX_TEST_PRIVATE", "PRYX_TEST_REMOTE", "PRYX_TEST_EDGE", "PRYX_TEST_HOST_ADDRESS"}
        if fields.keys() != required:
            raise ValueError("The stock destination fixture did not report its four endpoints")
        args = ["sudo", "--non-interactive", "env", "PRYX_TEST_AGENT=" + AGENT]
        args += [name + "=" + fields[name] for name in sorted(required)]
        status["phase"] = "run_stock_process_network"
        with log.open("w") as target:
            result = subprocess.run([*args, binary, "--ignored", "--test-threads", "1", "--nocapture"],
                                    cwd=checkout, stdout=target, stderr=subprocess.STDOUT)
        lines = log.read_text(errors="replace").splitlines()
        if result.returncode:
            raise subprocess.CalledProcessError(result.returncode, ["process_network"])
        status["phase"] = "validate_test_summary"
        if not any(re.match(rf"^test result: ok\. {len(tests)} passed; 0 failed;", line) for line in lines):
            raise ValueError("Missing complete stock test summary")
        for name in tests:
            if not any(line.startswith(f"test {name} ...") for line in lines):
                raise ValueError(f"Missing unchanged test result: {name}")
    finally:
        # Only this fresh native test runner may own these fixture names.
        names = output(["ip", "netns", "list"]).splitlines()
        if any(line.split()[0] == "pryx-private" for line in names):
            pids = output(["sudo", "ip", "netns", "pids", "pryx-private"]).split()
            if pids:
                subprocess.run(["sudo", "kill", "--", *pids], check=False)
            call(["sudo", "ip", "netns", "delete", "pryx-private"])
        if Path("/sys/class/net/pryx-host").exists():
            call(["sudo", "ip", "link", "del", "pryx-host"])
        if Path(AGENT).exists():
            call(["sudo", "rm", "--", AGENT])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("operation", choices=["run"])
    parser.add_argument("output", type=Path)
    parser.add_argument("--experiment", choices=["local-tcp-ack-first"])
    args = parser.parse_args()
    if (os.getuid() == 0 or os.environ.get("GITHUB_ACTIONS") != "true"
            or os.environ.get("GITHUB_REPOSITORY") != "cubepals/pryx"
            or os.environ.get("GITHUB_EVENT_NAME") != "workflow_dispatch"):
        parser.error("use an explicitly dispatched native disposable cubepals/pryx runner")
    root = HERE.parents[1]
    if output(["git", "-C", root, "rev-parse", "HEAD"]) != os.environ["GITHUB_SHA"]:
        raise ValueError("The experiment checkout differs from the dispatched revision")
    location = args.output.resolve()
    location.mkdir(mode=0o700)
    status = {"phase": "stage_stock_checkout", "tests": []}
    try:
        attempt(root, location, args.experiment, status)
    except Exception as error:
        record_failure(location, status, error, args.experiment)
        raise SystemExit("Policy acceptance failed; inspect failure.json and the private runner log") from None


def record_failure(location, status, error, experiment):
    # Permit only names enumerated by the pinned test executable, fixed phases and numbers.
    # Never copy test output, environment settings, endpoints or exception strings to this receipt.
    log = location / "process-network.log"
    lines = log.read_text(errors="replace").splitlines() if log.exists() else []
    failed = [name for name in status["tests"] if
              any(line.strip() == name or line == f"test {name} ... FAILED" or
                  line == f"---- {name} stdout ----" for line in lines)]
    receipt = {"stock_commit": STOCK, "lab_commit": os.environ["GITHUB_SHA"],
               "selection": experiment or "stock", "phase": status["phase"], "failed_cases": failed,
               "exit_status": error.returncode if isinstance(error, subprocess.CalledProcessError) else None}
    (location / "failure.json").write_text(json.dumps(receipt, indent=2) + "\n")


def attempt(root, location, experiment, status):
    with tempfile.TemporaryDirectory(dir=os.environ["RUNNER_TEMP"]) as scratch:
        private = Path(scratch)
        checkout = private / "stock"
        if subprocess.run(["git", "-C", root, "cat-file", "-e", STOCK + "^{commit}"],
                          stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode:
            call(["git", "-C", root, "fetch", "--no-tags", "--depth=1",
                  "https://github.com/cubepals/pryx.git", STOCK])
        call(["git", "-C", root, "worktree", "add", "--detach", checkout, STOCK])
        try:
            env = os.environ | {"CARGO_TARGET_DIR": str(checkout / "target"), "CARGO_INCREMENTAL": "0",
                                "CARGO_PROFILE_DEV_DEBUG": "0", "CARGO_PROFILE_TEST_DEBUG": "0"}
            status["phase"] = "build_and_identify_test_and_policy"
            binary, tests, stock, selected = build(checkout, private, env, experiment)
            status["tests"] = tests
            info = {"stock_commit": STOCK, "lab_commit": os.environ["GITHUB_SHA"],
                    "architecture": platform.machine(), "stock_object_sha256": sha256(stock),
                    "selection": experiment or "stock", "selected_object_sha256": sha256(selected), "test_binary_sha256": sha256(binary),
                    "agent_binary_sha256": sha256(checkout / "target/debug/pryx-agent"), "test_target": "process_network", "ignored_tests_discovered": tests}
            (location / "inventory.json").write_text(json.dumps(info, indent=2) + "\n")
            run_suite(checkout, binary, tests, private / "fixture", location / "process-network.log", status)
            info["passed"] = len(tests)
            (location / "passed.json").write_text(json.dumps(info, indent=2) + "\n")
        finally:
            call(["git", "-C", root, "worktree", "remove", "--force", checkout])


if __name__ == "__main__":
    main()
