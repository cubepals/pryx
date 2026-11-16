#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Acquire CI's pinned RustFS image through its official registries.

Docker owns registry authentication, TLS, digest checking and layer extraction. This wrapper
bounds idle clients and new attempts, checks the resulting platform/image identity, and keeps
the existing local test alias. It is not a registry client or a runtime image resolver.
"""

from decimal import Decimal
import errno
import os
import pty
import re
from pathlib import Path
import selectors
import signal
import subprocess
import sys
import tempfile
import termios
import time


IMAGE_ALIAS = "ghcr.io/rustfs/rustfs:1.0.0"
# RustFS 1.0.0's official Docker Hub and GHCR publications have identical platform manifests:
# https://github.com/rustfs/rustfs/blob/1.0.0/.github/workflows/docker.yml
REGISTRIES = (IMAGE_ALIAS.rsplit(":", 1)[0], "docker.io/rustfs/rustfs")
PLATFORMS = {
    "linux/amd64": (
        "sha256:ba0a1b53e36f321c0d46f3867104abef169f7bc59c467c664ddac87e7ddc9a8b",
        "sha256:1e81764caba3be454540c061eb50beeb7706b3c46ecec9b4243498b585760c1b",
    ),
    "linux/arm64": (
        "sha256:42edb61d588775f9431ff436216d14392d2234d4eb2ed68321569fbf7245b36b",
        "sha256:f962960e92fb1acdda148adcd2ed51af8d4cd5d4d2e9b1df644f6e3f6afe7b51",
    ),
}
# Layer IDs are the Docker renderer's short digests from the same pinned OCI manifests above.
LAYER_IDS = {
    "linux/amd64": (
        "55afa1ecc21d", "fc60adad4a1d", "2db50cf901eb", "eebc17ed47f7",
        "a43ede1efa93", "4f4fb700ef54", "e993fc96bab5",
    ),
    "linux/arm64": (
        "5de55e5ef9c0", "7798bc502249", "e6ebdd134ef4", "4cb7aba558c3",
        "a43ede1efa93", "4f4fb700ef54", "5136e6219441",
    ),
}
# Docker's go-units HumanSize renderer uses decimal SI units, not binary byte units:
# https://github.com/docker/go-units/blob/v0.5.0/size.go
SI_UNITS = {unit: 1000 ** power for power, unit in enumerate(("B", "kB", "MB", "GB", "TB", "PB", "EB"))}
# Fixed Docker download/extraction stages; waiting/retry/status text proves no forward work.
# https://github.com/moby/moby/blob/v28.0.4/distribution/xfer/download.go
STAGES = {"Downloading": (1, True), "Verifying Checksum": (2, False),
          "Download complete": (3, False), "Extracting": (4, True),
          "Pull complete": (5, False), "Already exists": (5, False)}
CSI = re.compile(rb"\x1b\[[0-?]*[ -/]*[@-~]")
LAYER_LINE = re.compile(
    r"^([0-9a-f]+): (" + "|".join(re.escape(status) for status in STAGES) +
    r")(?:\s+(\[[^\]]*\]\s*)?([0-9]+(?:\.[0-9]+)?)([kMGTPE]?B)/([0-9]+(?:\.[0-9]+)?)([kMGTPE]?B))?\s*$"
)
MAX_PROGRESS_LINE_BYTES = 8192


class Progress:
    """Track only monotonic work by this pinned image's bounded set of layer IDs."""

    def __init__(self, platform):
        self.layers = dict.fromkeys(LAYER_IDS[platform], (0, Decimal(0)))
        self.pending = b""
        self.overflow = False

    def feed(self, chunk):
        lines = re.split(rb"[\r\n]", self.pending + chunk)
        self.pending = lines.pop()
        if self.overflow:
            if not lines:
                self.pending = b""
                return False
            lines = lines[1:]
            self.overflow = False
        if len(self.pending) > MAX_PROGRESS_LINE_BYTES:
            self.pending = b""
            self.overflow = True
        advanced = False
        for line in lines:
            if len(line) > MAX_PROGRESS_LINE_BYTES:
                continue
            match = LAYER_LINE.fullmatch(CSI.sub(b"", line).decode("ascii", errors="replace"))
            if not match or match[1] not in self.layers:
                continue
            layer, status = match[1], match[2].strip()
            stage, numeric = STAGES[status]
            previous_stage, previous_bytes = self.layers[layer]
            current = Decimal(0)
            if numeric:
                if match[4] is None:
                    continue
                current = Decimal(match[4]) * SI_UNITS[match[5]]
                total = Decimal(match[6]) * SI_UNITS[match[7]]
                if current <= 0 or current > total:
                    continue
            elif match[4] is not None:
                continue
            if stage > previous_stage or (stage == previous_stage and current > previous_bytes):
                self.layers[layer] = (stage, current)
                advanced = True
        return advanced


# Local acquisition policy: a transfer that keeps reporting progress has no total deadline.
IDLE_SECONDS = 90
START_BUDGET_SECONDS = 300
BACKOFF_SECONDS = (2, 5)
CONTROL_SECONDS = 15
RETIRE_SECONDS = 5
_STOP_SIGNAL = 0


class AcquisitionError(Exception):
    pass


def check_stop():
    if _STOP_SIGNAL:
        raise SystemExit(128 + _STOP_SIGNAL)


def control(docker, *arguments):
    """Read only explicitly selected engine fields; never log arbitrary engine output."""
    check_stop()
    try:
        result = subprocess.run(
            [*docker, *arguments], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
            timeout=CONTROL_SECONDS, check=True, text=True,
        )
    except (OSError, subprocess.SubprocessError) as error:
        raise AcquisitionError("Docker image control operation failed") from error
    check_stop()
    return result.stdout.strip()


def pull(docker, source, platform):
    """Retire and reap this owned CLI on idle, cancellation, error or normal completion."""
    check_stop()
    master, terminal = pty.openpty()
    process = None
    try:
        # Docker suppresses intermediate download progress on pipes. An owned PTY asks its
        # maintained renderer for real layer progress, without exposing that output to logs.
        termios.tcsetwinsize(terminal, (24, 80))
        process = subprocess.Popen(
            [*docker, "pull", "--platform", platform, source],
            stdin=subprocess.DEVNULL, stdout=terminal, stderr=subprocess.STDOUT,
            start_new_session=True,
        )
        os.close(terminal)
        terminal = None
        with selectors.DefaultSelector() as events:
            events.register(master, selectors.EVENT_READ)
            progress = Progress(platform)
            progress_at = time.monotonic()
            while True:
                check_stop()
                remaining = IDLE_SECONDS - (time.monotonic() - progress_at)
                if remaining <= 0:
                    return False
                if not events.select(timeout=min(remaining, 1)):
                    continue
                # Discard output: registry errors can contain bearer or redirect URLs. Docker's
                # non-quiet pull stream reports progress while downloading/extracting layers.
                try:
                    chunk = os.read(master, 65536)
                except OSError as error:
                    if error.errno != errno.EIO:
                        raise
                    chunk = b""  # Linux PTY EOF after the last slave closes.
                if chunk:
                    if progress.feed(chunk):
                        progress_at = time.monotonic()
                else:
                    try:
                        success = process.wait(timeout=min(remaining, 1)) == 0
                        check_stop()
                        return success
                    except subprocess.TimeoutExpired:
                        continue
    finally:
        if process is not None and process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=RETIRE_SECONDS)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
        os.close(master)
        if terminal is not None:
            os.close(terminal)


def acquire(docker):
    platform = control(docker, "version", "--format", "{{.Server.Os}}/{{.Server.Arch}}")
    if platform not in PLATFORMS:
        raise AcquisitionError("RustFS test image has no pinned identity for this platform")
    manifest, expected_image = PLATFORMS[platform]
    deadline = time.monotonic() + START_BUDGET_SECONDS
    for round_number in range(len(BACKOFF_SECONDS) + 1):
        check_stop()
        if round_number:
            delay = BACKOFF_SECONDS[round_number - 1]
            if time.monotonic() + delay >= deadline:
                break
            time.sleep(delay)
        for registry in REGISTRIES:
            check_stop()
            if time.monotonic() >= deadline:
                break
            source = f"{registry}@{manifest}"
            print(f"Acquire pinned RustFS from {registry}", flush=True)
            if not pull(docker, source, platform):
                print("RustFS source attempt failed or became idle", flush=True)
                continue
            identity = control(docker, "image", "inspect", "--format", "{{.Id}}", source)
            if identity != expected_image:
                # A successful pull with the wrong identity is an integrity failure, not a
                # source outage. Do not try another registry or publish the local alias.
                raise AcquisitionError("RustFS image identity mismatch")
            control(docker, "image", "tag", expected_image, IMAGE_ALIAS)
            if control(docker, "image", "inspect", "--format", "{{.Id}}", IMAGE_ALIAS) != expected_image:
                raise AcquisitionError("RustFS local alias identity mismatch")
            print(f"Pinned RustFS ready for {platform}", flush=True)
            return
    raise AcquisitionError("RustFS acquisition exhausted its bounded source attempts")


def stop(signum, _frame):
    # Defer raising until the owned Popen has entered its cleanup scope.
    global _STOP_SIGNAL
    _STOP_SIGNAL = signum


def main():
    signal.signal(signal.SIGTERM, stop)
    signal.signal(signal.SIGINT, stop)
    # Both registries publish this image anonymously. An empty private CLI configuration keeps
    # this acquisition from loading runner registry credentials or invoking credential helpers.
    with tempfile.TemporaryDirectory(prefix="pryx-rustfs-docker-") as configuration:
        Path(configuration, "config.json").write_text("{}\n")
        acquire(("docker", "--config", configuration))
        check_stop()


if __name__ == "__main__":
    try:
        main()
    except (AcquisitionError, OSError) as error:
        # Keep unexpected client text (including OSError filenames) out of CI logs as well.
        print("RustFS acquisition failed: " + (
            str(error) if isinstance(error, AcquisitionError) else "Docker client unavailable"
        ), file=sys.stderr)
        sys.exit(1)
