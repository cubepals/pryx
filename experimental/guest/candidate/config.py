#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Build two fixed lab init selectors from the stock configurator and compare their full outputs."""
import argparse
import copy
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

REPO = Path(__file__).resolve().parents[3]
MODULE = REPO / "templates/vm-tools"
SOURCE = MODULE / "cmd/config/main.go"
ORIGINAL = '"console=ttyS0 reboot=k panic=1 pci=off rootfstype=erofs init=/init ro"'
VARIANTS = {"vm-config-control": "/lab-control", "vm-config-activation": "/lab-init"}


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def invoke(executable, args):
    result = subprocess.run([executable, *args], check=True, capture_output=True, timeout=15)
    return json.loads(result.stdout)


def corrected_boot(stock, init):
    candidate = copy.deepcopy(stock)
    boot = candidate["boot-source"]["boot_args"]
    before, separator, after = boot.partition(" -- ")
    init_words = [word for word in before.split() if word.startswith("init=")]
    if init_words != ["init=/init"] or before.count(" init=/init ") != 1 or after != "vm-init" or not separator:
        raise ValueError("unexpected stock init or guest arguments")
    candidate["boot-source"]["boot_args"] = before.replace(" init=/init ", f" init={init} ") + separator + after
    return candidate


def check(stock, tools, fixture):
    # The configurator reads only stat information. These regular private files
    # never hold a monitor, kernel image, actual guest memory or filesystem.
    fixture.mkdir(mode=0o700)
    files = {}
    for name in ("kernel", "root", "writes", "layer0", "channel"):
        files[name] = fixture / name
        files[name].write_bytes(name.encode("ascii"))

    args = ["--kernel", files["kernel"], "--root", files["root"], "--writes", files["writes"],
            "--layer", f"layer0={files['layer0']}", "--socket", files["channel"], "--init-mode", "vm-init",
            "--memory-mib", "256", "--vcpus", "2", "--tap", "labtap0", "--guest-mac", "02:00:00:00:00:17"]
    meta = invoke(stock, ["--metadata"])
    ids = invoke(stock, [*args, "--identities"])
    expected_ids = {}
    for key, name in ((meta["rootDrive"], "root"), (meta["writeDrive"], "writes"), ("layer0", "layer0")):
        stat = files[name].stat()
        expected_ids[key] = f"{stat.st_dev}{stat.st_rdev}{stat.st_ino}"
    if ids != expected_ids or len(set(ids.values())) != 3:
        raise ValueError("stock configuration does not identify the three fixture disks")
    stock_plans = [(extra, invoke(stock, [*args, *extra])) for extra in ([], ["--jail-root", fixture])]
    if stock_plans[0][1] == stock_plans[1][1]:
        raise ValueError("jail fixture failed to distinguish host and monitor paths")
    for name, binary in tools.items():
        if invoke(binary, ["--metadata"]) != meta or invoke(binary, [*args, "--identities"]) != expected_ids:
            raise ValueError(f"{name} changed transport metadata or backing identities")
        for extra, host in stock_plans:
            actual = invoke(binary, [*args, *extra])
            if actual != corrected_boot(host, VARIANTS[name]):
                raise ValueError(f"{name} changed a monitor field outside its init argument")


def build(go, stock, output):
    source = SOURCE.read_text()
    if source.count(ORIGINAL) != 1 or source.count("init=/init") != 1:
        raise ValueError("stock boot literal changed; recheck the fixed configurator overlay")
    go_version = json.loads((MODULE / "spike-inputs.json").read_text())["go"]["version"]
    env = {**os.environ, "GOTOOLCHAIN": "local", "CGO_ENABLED": "0", "GOWORK": "off", "GOFLAGS": ""}
    found = subprocess.check_output([go, "env", "GOVERSION"], cwd=MODULE, env=env, text=True).strip()
    if found != f"go{go_version}":
        raise ValueError("configurator build requires the repository's pinned Go")
    with tempfile.TemporaryDirectory(prefix="lab-init-config-", dir=output.parent) as scratch:
        stage = Path(scratch)
        tools = {}
        inputs = {}
        for name, init in VARIANTS.items():
            replacement = stage / f"{name}.go"
            replacement.write_text(source.replace(ORIGINAL, ORIGINAL.replace("init=/init", f"init={init}")))
            overlay = stage / f"{name}.json"
            overlay.write_text(json.dumps({"Replace": {str(SOURCE): str(replacement)}}))
            binary = stage / name
            subprocess.run([go, "build", "-mod=readonly", "-trimpath", f"-overlay={overlay}", "-o", binary,
                            "./cmd/config"], cwd=MODULE, env=env, check=True)
            tools[name] = binary
            inputs[name] = {"init": init, "sourceSha256": digest(replacement), "executableSha256": digest(binary)}
        check(stock, tools, stage / "jail")
        report = {"stockExecutableSha256": digest(stock), "originalSourceSha256": digest(SOURCE),
                  "goVersion": found, "variants": inputs, "transportMetadataEqual": True,
                  "hostAndJailConfigsMatchExceptInit": True, "threeDriveIdentitiesVerified": True}
        result = stage / "config-proof.json"
        result.write_text(json.dumps(report, indent=2) + "\n")
        # Reserve a new private directory only after both binaries passed the stock checks.
        output.mkdir(mode=0o700)
        try:
            for source_file in [*tools.values(), result]:
                os.replace(source_file, output / source_file.name)
        except BaseException:
            shutil.rmtree(output)
            raise


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--go", type=Path, required=True)
    parser.add_argument("--stock", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    build(args.go.resolve(), args.stock.resolve(), args.output.absolute())
