#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Exercise the actual CLI dispatch and installer with an isolated home and signed loopback release.

The no-up install uses a bootstrap fixture. The existing Any host jobs exercise the real host
bundle, presets and services; this test concentrates on opt-in routing and archive authentication.
"""

from concurrent.futures import ThreadPoolExecutor
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
    # A hosted runner may export real-user XDG and pryx paths. Every product invocation
    # belongs to this fixture user, including the stock bootstrap's subprocesses.
    env = {k: v for k, v in os.environ.items() if not k.startswith(("XDG_", "PRYX_"))}
    for name, suffix in {"XDG_CONFIG_HOME": ".config", "XDG_DATA_HOME": ".local/share",
                         "XDG_STATE_HOME": ".local/state", "XDG_CACHE_HOME": ".cache",
                         "XDG_RUNTIME_DIR": ".run"}.items():
        directory = home / suffix
        directory.mkdir(parents=True, mode=0o700)
        env[name] = str(directory)
    env.update(HOME=str(home), PRYX_PRESET="user", PRYX_NO_UP="1",
               PRYX_RELEASE_BASE_URL=f"http://127.0.0.1:{server.server_port}")
    key = tmp / "preview.pem"
    subprocess.run(["openssl", "genpkey", "-algorithm", "ED25519", "-out", key], check=True)
    pub = subprocess.check_output(["openssl", "pkey", "-in", key, "-pubout"]).decode()
    public_key = tmp / "preview.pub"
    public_key.write_text(pub)
    cli = tmp / "pryx.js"
    subprocess.run(["bun", "build", "--target=bun", "cli/src/main.ts",
                    "--define", "PRYX_PREVIEW_PUBLIC_KEY=" + json.dumps(pub), "--outfile", cli],
                   cwd=REPO, check=True)
    def run(args, ok=True, overrides=None, shell=False, diagnostic=None):
        cmd = ["sh", REPO / "deploy/install.sh", *args] if shell else ["bun", cli, *args]
        child = subprocess.run(cmd, env=env | (overrides or {}), capture_output=True, text=True)
        assert (child.returncode == 0) == ok, (cmd, child.stdout, child.stderr)
        if diagnostic:
            assert diagnostic in child.stderr, child.stderr
        return child.stdout

    name = f"pryx-lab-linux-{ARCH}.pyz"
    asset = source / name
    # Unit-only runs can exercise the handoff with true; CI supplies the native build and checks its output.
    native = os.environ.get("PRYX_LAB_TEST_BINARY")
    subprocess.run([sys.executable, HERE / "package.py", native or shutil.which("true"), asset,
                    "--arch", ARCH], check=True)
    def sign(version=VERSION, arch=ARCH):
        subprocess.run([sys.executable, "-B", HERE / "sign.py", asset, str(asset) + ".sig",
                        "--version", version, "--arch", arch, "--key", "/dev/stdin", "--public-key", public_key],
                       input=key.read_bytes(), check=True)
    sign()
    # A valid signature for an unrelated signer must never replace the current release signature.
    signature = Path(str(asset) + ".sig")
    previous_signature = signature.read_bytes()
    wrong_key = subprocess.check_output(["openssl", "genpkey", "-algorithm", "ED25519"])
    wrong = subprocess.run([sys.executable, "-B", HERE / "sign.py", asset, signature, "--version", VERSION,
                            "--arch", ARCH, "--key", "/dev/stdin", "--public-key", public_key],
                           input=wrong_key, capture_output=True)
    assert wrong.returncode and signature.read_bytes() == previous_signature
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
    probe = tmp / "probe/bin"
    probe.mkdir(parents=True)
    (probe / "pryxd").write_text("""#!/bin/sh
echo '{"architecture":"fixture-host"}'
""")
    (probe / "pryxd").chmod(0o755)
    run(["--check", "--preset", "server", "--irrelevant"], shell=True)
    assert server.paths == ["/SHA256SUMS", "/" + archive.name]
    root = home / ".pryx-experimental"
    assert not root.exists()
    run(["up", "--experimental", "--check"])
    run(["up", "--experimental", "--check"], ok=False, overrides={"XDG_CONFIG_HOME": str(tmp / "outside")})
    run(["up", "--experimental", "--check", "--feature", "unknown"], diagnostic="unverified until installation")
    for flags in [["--feature"], ["--feature", "--invalid"], ["--feature", "self-check", "--feature", "self-check"]]:
        run(["up", "--experimental", "--check", *flags], ok=False)
    assert len(server.paths) == 2 and not root.exists()
    run(["--experimental", "--feature", "unknown", "--feature", "self-check"], shell=True, ok=False)
    assert server.paths[2:] == ["/SHA256SUMS", "/" + archive.name] and not root.exists()

    run(["--experimental", "--run-installed", "--check", "--preset", "node"], shell=True)
    assert server.paths[-2:] == ["/" + name + ".sig", "/" + name]
    initial = (root / "active").resolve()
    assert (initial / "selection").read_text() == ""
    run(["experimental", "run", "self-check"], ok=False)
    (initial / "selection").write_text("self-check")
    run(["experimental", "run", "self-check"], ok=False, diagnostic="selection failed its content check")
    forged = json.loads(run(["doctor", "--json"], overrides={"PRYX_BUNDLE_DIR": str(probe.parent)}))
    assert forged["experimental"]["installed"] is False and "content check" in forged["experimental"]["reason"]
    run(["--feature", "self-check"], shell=True, overrides={"PRYX_EXPERIMENTAL": "1"})
    active = (root / "active").resolve()
    assert active != initial and (active / "selection").read_text() == "self-check"
    result = run(["experimental", "run", "self-check"])
    if native:
        assert result.strip() == "self-check passed", result
    run(["up", "--experimental", "--feature", "unknown"], ok=False)
    run(["up", "--experimental", "--feature", "self-check", "--preset", "node"], ok=False)
    assert (root / "active").resolve() == active
    report = json.loads(run(["capabilities"], overrides={"PRYX_BUNDLE_DIR": str(probe.parent)}))
    assert report["experimental"]["self-check"]["on"]
    for version, arch in [("999.0.0", ARCH), (VERSION, "arm64" if ARCH == "x64" else "x64")]:
        sign(version, arch)
        run(["experimental", "install", "--experimental"], ok=False)
        assert (root / "active").resolve() == active
    # A CLI upgrade can leave a healthy archive signed for the old release installed.
    sign("999.0.0")
    (active / "signature").write_bytes(signature.read_bytes())
    stale = json.loads(run(["doctor", "--json"], overrides={"PRYX_BUNDLE_DIR": str(probe.parent)}))
    assert stale["architecture"] == "fixture-host"
    assert stale["experimental"]["installed"] is False and "release" in stale["experimental"]["reason"]
    assert "pryx experimental install --experimental" in stale["experimental"]["action"]
    run(["experimental", "run", "self-check"], ok=False)
    (active / "signature").write_bytes(previous_signature)
    sign()
    authentic_archive = asset.read_bytes()
    asset.write_bytes(authentic_archive + b"changed")
    run(["experimental", "install", "--experimental"], ok=False)
    (active / "lab.pyz").write_bytes(b"changed")
    run(["experimental", "run", "self-check"], ok=False)
    off = json.loads(run(["doctor", "--json"], overrides={"PRYX_BUNDLE_DIR": str(probe.parent)}))
    assert off["experimental"]["installed"] is False and "signature" in off["experimental"]["reason"]
    asset.write_bytes(authentic_archive)
    repair = ["experimental", "install", "--experimental", "--feature", "self-check"]
    with ThreadPoolExecutor(max_workers=2) as workers:
        list(workers.map(run, [repair, repair]))
    repaired = (root / "active").resolve()
    assert repaired != active and (active / "lab.pyz").read_bytes() == b"changed"
    assert (repaired / "lab.pyz").read_bytes() == authentic_archive
    assert (repaired / "selection").read_text() == "self-check"
    run(repair)
    assert (root / "active").resolve() == repaired
    # The prior disabled generation was intentionally damaged above. Repair it once, then
    # alternate selectors and simulate a crash after preparing the enabled generation.
    run(["experimental", "install", "--experimental"])
    disabled = (root / "active").resolve()
    count = len(list(root.glob("build-*")))
    for _ in range(2):
        run(repair)
        assert (root / "active").resolve() == repaired
        run(["experimental", "install", "--experimental"])
        assert (root / "active").resolve() == disabled
    (root / "active").unlink()
    run(repair)
    assert (root / "active").resolve() == repaired and len(list(root.glob("build-*"))) == count
    repaired_report = json.loads(run(["doctor", "--json"], overrides={"PRYX_BUNDLE_DIR": str(probe.parent)}))
    assert repaired_report["experimental"]["self-check"]["on"]
    output = run(["experimental", "run", "self-check"])
    if native:
        assert output.strip() == "self-check passed"
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
