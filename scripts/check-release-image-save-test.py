# SPDX-License-Identifier: FSL-1.1-ALv2
"""Check the release image producer with an offline Docker save fixture."""

from itertools import takewhile
import os
from pathlib import Path
import subprocess
import tarfile
import tempfile
from textwrap import dedent
import unittest


DOCKER_SAVE_FAILURE = 23
WORKFLOW = Path(__file__).resolve().parents[1] / ".github/workflows/ci-acceptance.yml"


class ReleaseImageSave(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.directory = Path(temporary.name)
        self.artifact = self.directory / "ci-release-build"
        self.artifact.mkdir()
        self.binaries = self.directory / "bin"
        self.binaries.mkdir()
        docker = self.binaries / "docker"
        docker.write_text("""#!/bin/sh
set -eu
case "$1" in
    build|run|tag) ;;
    save)
        cat "$DOCKER_SAVE_FIXTURE"
        exit "$DOCKER_SAVE_STATUS"
        ;;
    *) exit 2 ;;
esac
""")
        docker.chmod(0o755)

        self.image = self.directory / "image.txt"
        self.image.write_text("fixture layer\n")
        self.tar = self.directory / "fixture.tar"
        with tarfile.open(self.tar, "w", format=tarfile.USTAR_FORMAT) as archive:
            archive.add(self.image, arcname=self.image.name)

        self.command = self.directory / "producer.sh"
        _, separator, body = WORKFLOW.read_text().partition(
            "      - name: Build the release image\n        run: |\n"
        )
        self.assertTrue(separator, "Release image producer step is missing its run block")
        lines = takewhile(lambda line: not line.strip() or line.startswith("          "),
                          body.splitlines(keepends=True))
        self.command.write_text(dedent("".join(lines)))

    def produce(self, fixture, status):
        environment = {
            "PATH": str(self.binaries) + os.pathsep + os.defpath,
            "RUNNER_TEMP": str(self.directory),
            "DOCKER_SAVE_FIXTURE": str(fixture),
            "DOCKER_SAVE_STATUS": str(status),
        }
        # GitHub's implicit Linux shell runs bash with -e.
        return subprocess.run(["bash", "-e", self.command], cwd=self.directory, env=environment,
                              capture_output=True, text=True, check=False)

    def test_failed_save_exits_before_writing_a_manifest(self):
        partial = self.directory / "partial.tar"
        partial.write_bytes(self.tar.read_bytes()[:tarfile.BLOCKSIZE + 1])
        result = self.produce(partial, DOCKER_SAVE_FAILURE)
        self.assertEqual((result.returncode, (self.artifact / "SHA256SUMS").exists()),
                         (DOCKER_SAVE_FAILURE, False), result.stderr)

    def test_complete_save_writes_a_readable_archive_and_manifest(self):
        result = self.produce(self.tar, 0)
        self.assertEqual(result.returncode, 0, result.stderr)
        subprocess.run(["sha256sum", "-c", "SHA256SUMS"], cwd=self.artifact, check=True,
                       capture_output=True)
        with tarfile.open(self.artifact / "pryx-release-image.tar.gz", "r:gz") as archive:
            self.assertEqual(archive.extractfile(self.image.name).read(), self.image.read_bytes())


if __name__ == "__main__":
    unittest.main(verbosity=2)
