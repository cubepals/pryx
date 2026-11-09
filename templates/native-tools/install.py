# SPDX-License-Identifier: FSL-1.1-ALv2
"""Install only audited, immutable Debian archives through dpkg; never resolve new packages."""
from concurrent.futures import ThreadPoolExecutor
import hashlib
import json
import pathlib
import subprocess
import tempfile
import urllib.request

with tempfile.TemporaryDirectory(prefix="pryx-debs-") as temporary:
    def download(package):
        path = pathlib.Path(temporary, package["package"] + ".deb")
        cached=pathlib.Path("/build-cache",package["archive"])
        if cached.is_file():data=cached.read_bytes()
        else:
            with urllib.request.urlopen(package["url"], timeout=20) as response:
                data = response.read()
        if hashlib.sha256(data).hexdigest() != package["sha256"]:
            raise SystemExit("Package hash mismatch: " + package["package"])
        path.write_bytes(data)
        print("Verified",package["package"],flush=True)
        return str(path)
    packages=json.loads(pathlib.Path(__file__).with_name("debian.lock.json").read_text())
    with ThreadPoolExecutor(max_workers=4) as pool:
        archives=list(pool.map(download,packages))
    # Daemons must never start or leave host keys in a build layer.
    policy = pathlib.Path("/usr/sbin/policy-rc.d")
    policy.write_text("#!/bin/sh\nexit 101\n")
    policy.chmod(0o755)
    subprocess.run(["dpkg", "--install", *archives], check=True)
    for key in pathlib.Path("/etc/ssh").glob("ssh_host_*"):
        key.unlink()
    policy.unlink()
    pathlib.Path("/run/sshd").mkdir(mode=0o755, exist_ok=True)
    pathlib.Path("/mnt").chmod(0o777)
    subprocess.run(["useradd", "--uid", "1000", "--home-dir", "/data", "--shell", "/bin/sh", "--password", "x", "pryx"], check=True)
    # Nested user namespaces supply mount authority; a setuid transition would discard ambient caps.
    # Execute-only, as the mount helpers that run it with their credentials in its environment.
    pathlib.Path("/bin/fusermount3").chmod(0o111)
