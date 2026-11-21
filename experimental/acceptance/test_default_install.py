# SPDX-License-Identifier: FSL-1.1-ALv2
"""Prove that the default-install audit detects payload and download changes."""
import io
from pathlib import Path
import tarfile
import tempfile
import unittest

from default_install import audit


class DefaultInstall(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        root = Path(tmp.name)
        self.base, self.release = root / "before", root / "after"
        self.base.mkdir()
        self.release.mkdir()
        self.files = {name: b"base data" for name in ("bin/pryxd", "licenses/notice")}
        self.files["bin/pryx.real"] = b"\x7fELFbase"
        self.write(self.base, self.files)
        self.write(self.release, {**self.files, "bin/pryx.real": b"\x7fELFnew cli"})
        self.log = root / "access.log"
        self.log.write_text("".join(f'"GET /{name} HTTP/1.1" 200 -\n' for name in (
            "install.sh", "SHA256SUMS", "pryx-bootstrap-linux-x64.tar.gz", "pryx-bundle-linux-x64.tar.gz")))

    def write(self, release, files):
        for stem in ("bootstrap", "bundle"):
            with tarfile.open(release / f"pryx-{stem}-linux-x64.tar.gz", "w:gz") as archive:
                for name, payload in sorted(files.items()):
                    info = tarfile.TarInfo(name)
                    info.size = len(payload)
                    archive.addfile(info, io.BytesIO(payload))

    def check(self):
        return audit(self.release, self.base, self.log, "x64")

    def test_allows_the_native_cli_handoff(self):
        self.assertEqual(self.check()["lab_requests"], 0)

    def test_rejects_any_other_payload_change(self):
        self.write(self.release, {**self.files, "bin/pryxd": b"changed daemon"})
        with self.assertRaisesRegex(ValueError, "bin/pryxd"):
            self.check()

    def test_rejects_a_failed_lab_request(self):
        with self.log.open("a") as log:
            log.write('"GET /pryx-lab-linux-x64.pyz HTTP/1.1" 404 -\n')
        with self.assertRaisesRegex(ValueError, "requested a lab asset"):
            self.check()

    def test_requires_both_archives(self):
        (self.release / "pryx-bootstrap-linux-x64.tar.gz").unlink()
        with self.assertRaises(FileNotFoundError):
            self.check()
