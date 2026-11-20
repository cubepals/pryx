#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Bound a maintained artifact client's idle time, not a progressing archive's total duration.

GitHub CLI v2.103.3 streams to TMPDIR/gh-artifact.*.zip before extracting files. Observe only
the private download and extraction directories; CLI handles HTTP, credentials and safe ZIP
extraction. This wrapper neither reads their contents nor changes content authentication.
"""
import argparse
from contextlib import contextmanager
import math
import os
from pathlib import Path
import signal
import stat
import subprocess
import sys
import tempfile
import threading
import time

# GNU timeout(1)'s documented timeout exit status, retained for callers of the former wrapper.
TIMEOUT_EXIT_STATUS = 124


def written_bytes(*directories):
    total = 0
    for directory in directories:
        for root, _, files in os.walk(directory, followlinks=False):
            for name in files:
                try:
                    info = os.stat(Path(root) / name, follow_symlinks=False)
                except FileNotFoundError:
                    continue  # The client removes its temporary archive after extraction.
                if stat.S_ISREG(info.st_mode):
                    total += info.st_size
    return total


def stop(process):
    try:
        os.killpg(process.pid, signal.SIGTERM)
    except ProcessLookupError:
        pass
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        process.wait()


@contextmanager
def defer_cancellation():
    """Check cancellation while watching the client, and finish reaping before unwinding."""
    pending = False
    deferred = []

    def remember(_signal, _frame):
        nonlocal pending
        pending = True

    def check():
        nonlocal pending
        if pending:
            pending = False
            raise KeyboardInterrupt

    try:
        if threading.current_thread() is threading.main_thread():
            for sig in (signal.SIGINT, signal.SIGTERM):
                if signal.getsignal(sig) is signal.default_int_handler:
                    deferred.append(sig)
                    signal.signal(sig, remember)
        yield check
    finally:
        if deferred:
            # Client cleanup has finished. Block only while restoring both handlers so a
            # second signal cannot interrupt a partial restore or change the child's mask.
            mask = signal.pthread_sigmask(signal.SIG_BLOCK, deferred)
            try:
                for sig in deferred:
                    signal.signal(sig, signal.default_int_handler)
            finally:
                signal.pthread_sigmask(signal.SIG_SETMASK, mask)
        check()


def read(command, stage, log, idle_seconds):
    started = time.monotonic()
    deadline = started + idle_seconds
    most_bytes = 0
    with tempfile.TemporaryDirectory(prefix='gh-artifact-read-', dir=stage.parent) as temporary:
        # Go's os.CreateTemp("", ...) uses TMPDIR; the pinned CLI's http.go owns this layout.
        with log.open('w') as output, defer_cancellation() as check_cancellation:
            process = None
            try:
                # The client's diagnostics can include a temporary download URL.
                process = subprocess.Popen(command, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                                           start_new_session=True, env={**os.environ, 'TMPDIR': temporary})
                while process.poll() is None:
                    check_cancellation()
                    progress = written_bytes(stage, temporary)
                    now = time.monotonic()
                    if progress > most_bytes:
                        most_bytes = progress
                        deadline = now + idle_seconds
                    if now >= deadline:
                        stop(process)
                        output.write(f'Artifact read idle timeout: {idle_seconds:g}s without file '
                                     f'progress; observed_bytes={most_bytes}\n')
                        return TIMEOUT_EXIT_STATUS
                    try:
                        process.wait(timeout=min(0.25, deadline - now))
                    except subprocess.TimeoutExpired:
                        pass
            finally:
                if process is not None and process.poll() is None:
                    stop(process)
            if process.returncode:
                output.write(f'Artifact read exited {process.returncode}; '
                             f'observed_bytes={most_bytes}; '
                             f'elapsed_seconds={time.monotonic() - started:.2f}\n')
            return process.returncode


def main():
    # A cancelled runner must unwind read() and stop its client, as Ctrl-C already does.
    signal.signal(signal.SIGTERM, signal.default_int_handler)
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('stage', type=Path)
    parser.add_argument('log', type=Path)
    parser.add_argument('idle_seconds', type=float)
    parser.add_argument('command', nargs=argparse.REMAINDER)
    options = parser.parse_args()
    if not math.isfinite(options.idle_seconds) or options.idle_seconds <= 0 or not options.command:
        parser.error('a positive idle interval and client command are required')
    return read(options.command, options.stage, options.log, options.idle_seconds)


if __name__ == '__main__':
    sys.exit(main())
