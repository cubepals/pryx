#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Exercise the actual CLI dispatch and installer with an isolated home and signed loopback release.

The no-up install uses a bootstrap fixture. The existing Any host jobs exercise the real host
bundle, presets and services; this test concentrates on opt-in routing and archive authentication.
"""

from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import tarfile
import tempfile
import threading
import tomllib

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
ARCH = {"x86_64": "x64", "aarch64": "arm64"}[platform.machine()]
VERSION = tomllib.loads((REPO / "Cargo.toml").read_text())["workspace"]["package"]["version"]


class Recorder(SimpleHTTPRequestHandler):
    def log_message(self, *args):
        self.server.paths.append(self.path)


def exercise(tmp, server):
    source, home = tmp / "release", tmp / "home"
    home.mkdir(mode=0o700)
    env = {**os.environ, "HOME": str(home), "PRYX_PRESET": "user", "PRYX_NO_UP": "1",
           "PRYX_RELEASE_BASE_URL": f"http://127.0.0.1:{server.server_port}"}
    env.pop("PRYX_EXPERIMENTAL", None)
    key = tmp / "preview.pem"
    subprocess.run(["openssl", "genpkey", "-algorithm", "ED25519", "-out", key], check=True)
    pub = subprocess.check_output(["openssl", "pkey", "-in", key, "-pubout"]).decode()
    cli = tmp / "pryx.js"
    subprocess.run(["bun", "build", "--target=bun", "cli/src/main.ts",
                    "--define", "PRYX_PREVIEW_PUBLIC_KEY=" + json.dumps(pub), "--outfile", cli],
                   cwd=REPO, check=True)
    def run(args, ok=True, overrides=None, shell=False):
        cmd = ["sh", REPO / "deploy/install.sh", *args] if shell else ["bun", cli, *args]
        child = subprocess.run(cmd, env=env | (overrides or {}), capture_output=True, text=True)
        assert (child.returncode == 0) == ok, (cmd, child.stdout, child.stderr)
        return child.stdout

    name = f"pryx-lab-linux-{ARCH}.pyz"
    asset = source / name
    # Unit-only runs can exercise the handoff with true; CI supplies the native build and checks its output.
    native = os.environ.get("PRYX_LAB_TEST_BINARY")
    subprocess.run([sys.executable, HERE / "package.py", native or shutil.which("true"), asset,
                    "--arch", ARCH], check=True)
    def sign(version=VERSION, arch=ARCH):
        subprocess.run([sys.executable, "-B", HERE / "sign.py", asset, str(asset) + ".sig",
                        "--version", version, "--arch", arch, "--key", key], check=True)
    sign()
    boot = tmp / "bootstrap/bin"
    boot.mkdir(parents=True)
    wrapper = boot / "pryx"
    # The fixture only replaces the initial bootstrap extractor/installer. Installed commands are the stock CLI.
    wrapper.write_text('#!/bin/sh\nif [ "$1" = bootstrap-install ]; then mkdir -p "$(dirname "$4")"; '
                       'cp "$0" "$4"; exit; fi\nexec bun "$PRYX_TEST_CLI" "$@"\n')
    wrapper.chmod(0o755)
    env["PRYX_TEST_CLI"] = str(cli)
    archive = source / f"pryx-bootstrap-linux-{ARCH}.tar.gz"
    with tarfile.open(archive, "w:gz") as tar:
        tar.add(boot, "bin")
    (source / "SHA256SUMS").write_text(f"{hashlib.sha256(archive.read_bytes()).hexdigest()}  {archive.name}\n")
    run([], shell=True)
    assert server.paths == ["/SHA256SUMS", "/" + archive.name]
    root = home / ".pryx-experimental"
    assert not root.exists()
    run(["up", "--experimental", "--check"])
    for flags in [["--feature"], ["--feature", "unknown"], ["--feature", "self-check", "--feature", "self-check"]]:
        run(["up", "--experimental", "--check", *flags], ok=False)
    run(["--experimental", "--feature", "unknown", "--feature", "self-check"], shell=True, ok=False)
    assert len(server.paths) == 2 and not root.exists()

    run(["--experimental"], shell=True)
    assert server.paths[-2:] == ["/" + name + ".sig", "/" + name]
    initial = (root / "active").resolve()
    assert (initial / "selection").read_text() == ""
    run(["experimental", "run", "self-check"], ok=False)
    run(["--feature", "self-check"], shell=True, overrides={"PRYX_EXPERIMENTAL": "1"})
    active = (root / "active").resolve()
    assert active != initial and (active / "selection").read_text() == "self-check"
    result = run(["experimental", "run", "self-check"])
    if native:
        assert result.strip() == "self-check passed", result
    run(["up", "--experimental", "--feature", "unknown"], ok=False)
    run(["up", "--experimental", "--feature", "self-check", "--preset", "node"], ok=False)
    assert (root / "active").resolve() == active
    probe = tmp / "probe/bin"
    probe.mkdir(parents=True)
    (probe / "pryxd").write_text("#!/bin/sh\necho '{}'")
    (probe / "pryxd").chmod(0o755)
    report = json.loads(run(["capabilities"], overrides={"PRYX_BUNDLE_DIR": str(probe.parent)}))
    assert report["experimental"]["self-check"]["on"]
    for version, arch in [("999.0.0", ARCH), (VERSION, "arm64" if ARCH == "x64" else "x64")]:
        sign(version, arch)
        run(["experimental", "install", "--experimental"], ok=False)
        assert (root / "active").resolve() == active
    sign()
    asset.write_bytes(asset.read_bytes() + b"changed")
    run(["experimental", "install", "--experimental"], ok=False)
    (active / "lab.pyz").write_bytes(b"changed")
    run(["experimental", "run", "self-check"], ok=False)
    off = json.loads(run(["doctor", "--json"], overrides={"PRYX_BUNDLE_DIR": str(probe.parent)}))
    assert off["experimental"]["installed"] is False and "signature" in off["experimental"]["reason"]
    print("passed stock installer and CLI opt-in, preflight, rollback, signature and capability checks")


if __name__ == "__main__":
    if os.getuid() == 0:
        sys.exit("run as the ordinary user to exercise the home-directory installer")
    with tempfile.TemporaryDirectory() as directory:
        tmp = Path(directory)
        (tmp / "release").mkdir()
        server = ThreadingHTTPServer(("127.0.0.1", 0), partial(Recorder, directory=tmp / "release"))
        server.paths = []
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            exercise(tmp, server)
        finally:
            server.shutdown()
            server.server_close()
