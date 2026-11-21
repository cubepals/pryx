# SPDX-License-Identifier: FSL-1.1-ALv2
"""Check Go acquisition diagnostics with inert command results."""
from contextlib import redirect_stderr
from http import HTTPStatus
import io
import json
from pathlib import Path
import re
from subprocess import CompletedProcess
import unittest
from unittest.mock import patch

from go_acquisition import archive_retry, diagnostic_headings, failure, module_archive, module_list


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

    def test_response_detail_preserves_go_heading_boundaries(self):
        first = 'Reading the fixture archive'
        following = 'Reading the fixture checksum'
        text = '\n'.join([first, '\tserver response:', '\tThe archive is being prepared.\rPlease wait.',
                          '\tThe next update will follow.', following])
        self.assertEqual(list(diagnostic_headings(text)), [first, following])

    def test_direct_vcs_detail_is_skipped_without_dropping_import_headings(self):
        vcs = 'go: fixture module: git status in /fixture/source cache: exit status 1:'
        detail = '\tThe fixture source is currently unavailable.'
        status = HTTPStatus.SERVICE_UNAVAILABLE
        import_heading = f'\timports fixture: reading https://module-proxy.invalid: {status.value} {status.phrase}'
        text = '\n'.join([vcs, detail, 'package fixture', import_heading])
        self.assertEqual(list(diagnostic_headings(text)), [vcs, 'package fixture', import_heading])
        self.assertEqual(failure(import_heading), (True, f'HTTP {status.value}'))
        result = CompletedProcess([], 1, b'', text.encode())
        output = io.StringIO()
        with (patch('go_acquisition.subprocess.run', return_value=result) as run,
              patch('go_acquisition.time.sleep') as sleep, redirect_stderr(output)):
            with self.assertRaisesRegex(RuntimeError, 'direct VCS failure; attempts=1$'):
                module_list(Path('fixture-go'), Path('.'), ['./...'])
        run.assert_called_once()
        sleep.assert_not_called()
        self.assertEqual(output.getvalue(), '')

    def test_earlier_heading_owns_its_detail_boundary_and_retry_reason(self):
        unavailable = HTTPStatus.SERVICE_UNAVAILABLE
        refused = HTTPStatus.FORBIDDEN
        http_first = f'archive {unavailable.value}, fixture command:'
        vcs_first = f'fixture command for archive {unavailable.value}:'
        following = '\tThe fixture source is being prepared.'
        # Replace the format matchers with inert headings to test their ordering in isolation.
        with (patch('go_acquisition.HTTP_STATUS', re.compile(r'archive ([0-9]{3})')),
              patch('go_acquisition.VCS_RUN_ERROR', re.compile(r'fixture command.*(:)$'))):
            self.assertEqual(list(diagnostic_headings(f'{http_first}\n{following}')), [http_first, following])
            self.assertEqual(failure(http_first), (True, f'HTTP {unavailable.value}'))
            self.assertEqual(list(diagnostic_headings(f'{vcs_first}\n{following}')), [vcs_first])
            self.assertEqual(failure(vcs_first), (False, 'direct VCS failure'))
            self.assertEqual(failure(vcs_first, f'archive {refused.value}'), (False, f'HTTP {refused.value}'))

    def test_structured_http_refusal_remains_terminal(self):
        status = HTTPStatus.FORBIDDEN
        diagnostic = f'reading https://module-proxy.invalid: {status.value} {status.phrase}'
        result = CompletedProcess([], 1, json.dumps({'Error': diagnostic}).encode(), b'')
        output = io.StringIO()
        with (patch('go_acquisition.subprocess.run', return_value=result) as run,
              patch('go_acquisition.time.sleep') as sleep, redirect_stderr(output)):
            with self.assertRaisesRegex(RuntimeError, f'HTTP {status.value}; attempts=1$'):
                module_archive(Path('fixture-go'), Path('.'), 'fixture-module')
        run.assert_called_once()
        sleep.assert_not_called()
        self.assertEqual(output.getvalue(), '')
