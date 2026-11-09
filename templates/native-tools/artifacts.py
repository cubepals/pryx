# SPDX-License-Identifier: FSL-1.1-ALv2
"""Verify official tool archives before delegating extraction to standard archive owners."""
import hashlib
import json
import pathlib
import subprocess
import tempfile
import urllib.request
import zipfile

root = pathlib.Path(__file__).parent
for artifact in json.loads((root / "artifacts.json").read_text()):
    cached=pathlib.Path("/build-cache",artifact["name"]+".download")
    if cached.is_file():data=cached.read_bytes()
    else:
        with urllib.request.urlopen(artifact["url"],timeout=30) as response:
            data=response.read()
    if hashlib.sha256(data).hexdigest() != artifact["sha256"]:
        raise SystemExit("Artifact hash mismatch: " + artifact["name"])
    with tempfile.NamedTemporaryFile() as archive:
        archive.write(data)
        archive.flush()
        if artifact["name"] == "mountpoint":
            # Keep the release's complete notices; fuse3 supplies the helper, audited libfuse2 the ABI.
            subprocess.run(["dpkg-deb", "--extract", archive.name, "/"], check=True)
            pathlib.Path("/usr/local/bin/mount-s3").symlink_to("/opt/aws/mountpoint-s3/bin/mount-s3")
            pathlib.Path("/usr/local/bin/fusermount").symlink_to("/bin/fusermount3")
            # Execute-only keeps the helper non-dumpable after exec, so sandbox commands of the
            # same uid cannot read the credentials in its /proc environ; pryx refuses it readable.
            pathlib.Path("/opt/aws/mountpoint-s3/bin/mount-s3").chmod(0o111)
        elif artifact["name"] == "rclone":
            with zipfile.ZipFile(archive.name) as zipped:
                binary = zipped.read("rclone-v1.76.0-linux-amd64/rclone")
            target = pathlib.Path("/usr/local/bin/rclone")
            target.write_bytes(binary)
            # Execute-only, like mount-s3: its /proc environ holds the mount's credentials.
            target.chmod(0o111)
        else:
            target = pathlib.Path("/opt/pryx/novnc")
            target.mkdir(parents=True, exist_ok=True)
            subprocess.run(["tar", "--extract", "--gzip", "--file", archive.name, "--strip-components=1", "--directory", str(target)], check=True)
