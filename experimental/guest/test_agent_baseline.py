# SPDX-License-Identifier: FSL-1.1-ALv2
import unittest

from baseline_summary import summarize


def fixture():
    return {'complete': True, 'samples': [
        {'sample': i + 1, 'acceptance': {'signedCleanShutdown': True, 'authenticatedInitShutdown': True,
                                         'shutdownReplayRefused': True},
         'observation': {'launchToMonitorPlanSentNs': 1, 'launchToBootstrapDeliveredNs': 20 + i,
                         'launchToAuthenticatedReadyNs': 40 + i, 'bootstrapDeliveredToAuthenticatedReadyNs': 20,
                         'readyToRssExecCompletedNs': 9, 'guestInitRssKiB': 2001, 'guestServingAgentRssKiB': 3002,
                         'invalidBearerHealthRefused': True}} for i in range(15)]}


class BaselineSummary(unittest.TestCase):
    def test_upper_bound_keeps_the_observed_boot_prefix_and_roles_are_distinct(self):
        table = summarize(fixture())
        self.assertIn('42.55%', table)
        self.assertIn('Guest PID 1 RSS | 2001.000', table)
        self.assertIn('Guest serving agent RSS | 3002.000', table)

    def test_rejects_clock_mismatch_and_unaccepted_shutdown(self):
        report = fixture()
        report['samples'][3]['observation']['bootstrapDeliveredToAuthenticatedReadyNs'] += 1
        with self.assertRaises(ValueError):
            summarize(report)
        report = fixture()
        report['samples'][4]['acceptance']['shutdownReplayRefused'] = False
        with self.assertRaises(ValueError):
            summarize(report)
