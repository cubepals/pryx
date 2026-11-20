# SPDX-License-Identifier: FSL-1.1-ALv2
"""Check Go acquisition diagnostics with inert command results."""
from contextlib import redirect_stderr
from http import HTTPStatus
import io
import json
from pathlib import Path
from subprocess import CompletedProcess
import unittest
from unittest.mock import patch

from go_acquisition import archive_retry, module_list


class Diagnostics(unittest.TestCase):
    def test_transient_failure_logs_only_the_retry_summary(self):
        unavailable = HTTPStatus.SERVICE_UNAVAILABLE
        detail = 'The fixture archive will be available shortly.'
        error = (f'go: reading https://module-proxy.invalid: {unavailable.value} {unavailable.phrase}\n'
                 f'\tserver response: {detail}\n')
        output = '{"Module": null}\n'
        attempts = iter([(1, '', error), (0, output, '')])

        def command_result(command, *, text=False, **_options):
            status, stdout, stderr = next(attempts)
            if not text:
                stdout, stderr = stdout.encode(), stderr.encode()
            return CompletedProcess(command, status, stdout, stderr)

        diagnostics = io.StringIO()
        with (patch('go_acquisition.subprocess.run', side_effect=command_result) as run,
              patch('go_acquisition.time.sleep') as sleep, redirect_stderr(diagnostics)):
            self.assertEqual(module_list(Path('fixture-go'), Path('.'), ['./...']), output)

        self.assertEqual(run.call_count, 2)
        sleep.assert_called_once_with(archive_retry.FIRST_RETRY_WAIT)
        self.assertNotIn(detail, diagnostics.getvalue())
        self.assertEqual(json.loads(diagnostics.getvalue()), {
            'goModuleAcquisition': 'retry', 'reason': f'HTTP {unavailable.value}',
            'attempt': 1, 'waitSeconds': archive_retry.FIRST_RETRY_WAIT,
        })
