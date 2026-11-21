# SPDX-License-Identifier: FSL-1.1-ALv2
import tempfile
import unittest
from pathlib import Path

from boot import STAMP
from cohort import percentiles, schedule, summarize


class GuestSamples(unittest.TestCase):
    def test_serial_marker_requires_the_complete_timestamp(self):
        marker = b'pryx-lab-pid1 0000000000000079'
        self.assertIsNone(STAMP.search(b'prefix' + marker + b'\n'))
        self.assertIsNone(STAMP.search(marker))
        self.assertEqual(int(STAMP.search(b'linux\r\n' + marker + b'\r\n').group(1), 16), 121)

    def test_every_pair_changes_direction_on_every_round(self):
        for labels in (['agent','asm','c','agent-quiet','asm-quiet','c-quiet'], ['control','no-print-time']):
            trials = [(name, object()) for name in labels]
            rows = list(schedule(trials))
            self.assertEqual(len(rows), 15 * len(labels))
            last = {}
            for number in range(1,16):
                events = [(event, trial) for event, trial in rows if event['round'] == number]
                expected = labels if number % 2 else labels[::-1]
                self.assertEqual([event['variant'] for event, _ in events], expected)
                self.assertEqual([trial[0] for _, trial in events], expected)
                self.assertEqual([event['position'] for event, _ in events], list(range(1, len(labels) + 1)))
                self.assertEqual({event['direction'] for event, _ in events}, {'forward' if number % 2 else 'reverse'})
                order = {name: i for i, name in enumerate(expected)}
                for i, left in enumerate(labels):
                    for right in labels[i+1:]:
                        earlier = order[left] < order[right]
                        if number > 1:
                            self.assertNotEqual(earlier, last[left,right])
                        last[left,right] = earlier

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
        self.assertIn('| fast | 90.000 / 90.000 | 110.000 / 110.000 | — | — | — | pass |', table)
        self.assertIn('| slow-tail | 89.000 / 106.000 | 109.000 / 126.000 | — | — | — | fail |', table)

    def test_serial_bootstrap_phase_uses_only_ordered_host_observations(self):
        rows = [dict(variant='agent', pid1BootNs=90000000, launchToPid1SerialNs=110000000,
                     launchToBootstrapNs=110000000 + delta, serialToBootstrapNs=delta,
                     serialAndBootstrapSamePoll=same) for delta, same in
                [(2000000, False), (3000000, False), (100, True), (-300, False)]]
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory)
            summarize(rows, target, ['agent'], 'agent')
            self.assertIn(' | 2.500 / 3.000 | 2 | baseline |', (target / 'table.md').read_text())
