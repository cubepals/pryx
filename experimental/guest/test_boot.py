# SPDX-License-Identifier: FSL-1.1-ALv2
import tempfile
import unittest
from pathlib import Path

from boot import STAMP
from cohort import percentiles, summarize


class GuestSamples(unittest.TestCase):
    def test_serial_marker_requires_the_complete_timestamp(self):
        marker = b'pryx-lab-pid1 0000000000000079'
        self.assertIsNone(STAMP.search(b'prefix' + marker + b'\n'))
        self.assertIsNone(STAMP.search(marker))
        self.assertEqual(int(STAMP.search(b'linux\r\n' + marker + b'\r\n').group(1), 16), 121)

    def test_fifteen_samples_use_maximum_for_nearest_rank_p95(self):
        self.assertEqual(percentiles(range(1, 16)), (8, 15))

    def test_a_tail_regression_rejects_a_fast_median(self):
        rows = []
        for i in range(15):
            for name, duration in [('control', 100), ('fast', 90), ('slow-tail', 106 if i == 14 else 89)]:
                rows.append({'variant': name, 'pid1BootNs': duration * 1000000,
                             'launchToPid1SerialNs': (duration + 20) * 1000000})
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory)
            summarize(rows, target, ['control', 'fast', 'slow-tail'], 'control')
            table = (target / 'table.md').read_text()
        self.assertIn('| fast | 90.000 / 90.000 | 110.000 / 110.000 | — | pass |', table)
        self.assertIn('| slow-tail | 89.000 / 106.000 | 109.000 / 126.000 | — | fail |', table)
