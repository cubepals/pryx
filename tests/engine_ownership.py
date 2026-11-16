#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Bounded failure controls for the ownership diagnostic's FUSE resource lifetime.

Only temporary ordinary directories are real. Helpers, mounts and process signals are stubs;
the cases prove cleanup after setup fails, rather than claiming filesystem capability.
"""
import importlib.util
import io
from pathlib import Path
import tempfile
import time
from types import SimpleNamespace
import unittest
from unittest.mock import patch

SOURCE = Path(__file__).resolve().parents[1] / 'scripts' / 'engine-ownership-second.py'
SPEC = importlib.util.spec_from_file_location('ownership_probe', SOURCE)
PROBE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PROBE)


class Child:
    pid = 12345
    returncode = None

    def poll(self):
        return self.returncode

    def wait(self, timeout):
        if timeout > 2:
            raise AssertionError('helper wait must retain its cleanup bound')
        self.returncode = 0
        return self.returncode


class FailureProbe(PROBE.Probe):
    def __init__(self, negative=PROBE.NOT_MOUNTPOINT, unmount_error=False, missing_tool=False, readiness_error=None):
        super().__init__()
        self.deadline = time.monotonic()+1
        self.negative = negative
        self.unmount_error = unmount_error
        self.missing_tool = missing_tool
        self.readiness_error = readiness_error
        self.mount_checks = 0
        self.unmounts = 0
        self.observations = 0

    def run(self, argv, timeout=PROBE.COMMAND_LIMIT, fds=(), cleanup=False):
        if '--version' in argv:
            return {'exit': None if self.missing_tool else 0, 'timedOut': False}
        if '-u' in argv:
            self.unmounts += 1
            if not cleanup or timeout > 3:
                raise AssertionError('unmount must use reserved bounded cleanup')
            if self.unmount_error:
                raise OSError('controlled unmount subprocess failure')
            return {'exit': 0, 'timedOut': False}
        if 'mountpoint' in argv:
            self.mount_checks += 1
            if self.mount_checks == 1:
                return {'exit': PROBE.NOT_MOUNTPOINT, 'timedOut': False}
            if not cleanup:
                if self.readiness_error:
                    raise self.readiness_error
                raise PROBE.Missing('controlled readiness deadline')
            self.observations += 1
            return {'exit': self.negative, 'timedOut': False}
        raise AssertionError('unexpected fixture command')


class SetupCleanup(unittest.TestCase):
    def attempt(self, probe, expected, spawn_error=None, masked=False):
        child, log, signals = Child(), io.BytesIO(), []
        peer = SimpleNamespace(fds=[])
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            with patch.object(PROBE.tempfile, 'TemporaryFile', return_value=log) as logfile, \
                 patch.object(PROBE.subprocess, 'Popen', side_effect=spawn_error, return_value=child) as start, \
                 patch.object(PROBE.os, 'killpg', side_effect=lambda pid, sig: signals.append((pid, sig))):
                with self.assertRaises(expected):
                    try:
                        PROBE.mount_view(probe, peer, None, Path('/unused-helper'), base, 1000, 1000)
                    finally:
                        if masked:
                            # A failed TemporaryDirectory exit can replace the cleanup exception.
                            raise OSError('controlled later temporary-directory teardown failure')
                if probe.missing_tool:
                    logfile.assert_not_called()
        return child, log, signals, start.call_count

    def test_readiness_deadline_retires_started_helper_and_closes_log(self):
        probe = FailureProbe()
        child, log, signals, starts = self.attempt(probe, PROBE.Missing)
        self.assertEqual(starts, 1)
        self.assertEqual(probe.unmounts, 1)
        self.assertEqual(probe.observations, 1)
        self.assertEqual(child.returncode, 0)
        self.assertTrue(log.closed)
        self.assertEqual(signals, [(child.pid, PROBE.signal.SIGTERM)])
        self.assertFalse(probe.violation)

    def test_spawn_failure_closes_unpublished_log(self):
        probe = FailureProbe()
        _, log, signals, starts = self.attempt(probe, OSError, spawn_error=OSError('controlled spawn failure'))
        self.assertEqual(starts, 1)
        self.assertTrue(log.closed)
        self.assertEqual(signals, [])
        self.assertEqual(probe.unmounts, 0)

    def test_readiness_subprocess_failure_also_retires_started_helper(self):
        probe = FailureProbe(readiness_error=OSError('controlled readiness subprocess failure'))
        child, log, signals, starts = self.attempt(probe, OSError)
        self.assertEqual(starts, 1)
        self.assertEqual(probe.unmounts, 1)
        self.assertEqual(probe.observations, 1)
        self.assertEqual(child.returncode, 0)
        self.assertTrue(log.closed)
        self.assertEqual(len(signals), 1)
        self.assertFalse(probe.violation)

    def test_missing_tool_refuses_before_any_helper_or_log(self):
        probe = FailureProbe(missing_tool=True)
        _, log, signals, starts = self.attempt(probe, PROBE.Missing)
        self.assertEqual(starts, 0)
        self.assertEqual(signals, [])
        self.assertEqual(probe.unmounts, 0)
        self.assertFalse(log.closed, 'the uncreated stub log was never adopted')
        log.close()

    def test_unknown_mount_observation_cannot_be_reported_as_retired(self):
        for status in (None, 0, 1):
            with self.subTest(status=status):
                probe = FailureProbe(negative=status)
                child, log, signals, _ = self.attempt(probe, PROBE.Violation)
                self.assertEqual(child.returncode, 0)
                self.assertTrue(log.closed)
                self.assertEqual(len(signals), 1)
                self.assertTrue(probe.violation)
                self.assertIn('negative mount observation', probe.results[-1]['invariantFailure'])

    def test_unmount_exception_still_attempts_termination_observation_and_log_close(self):
        probe = FailureProbe(unmount_error=True)
        child, log, _, _ = self.attempt(probe, PROBE.Violation)
        self.assertEqual(child.returncode, 0)
        self.assertEqual(probe.observations, 1)
        self.assertTrue(log.closed)
        self.assertTrue(probe.violation)

    def test_later_filesystem_failure_cannot_downgrade_cleanup_violation(self):
        probe = FailureProbe(negative=1)
        _, log, _, _ = self.attempt(probe, OSError, masked=True)
        self.assertTrue(probe.violation)
        self.assertTrue(log.closed)
        self.assertIn('unconfirmed', probe.results[-1]['invariantFailure'])


if __name__ == '__main__':
    unittest.main()
