#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Summarize an accepted A3 baseline and bound changes confined to a single phase."""
import argparse
import json
from pathlib import Path

from cohort import percentiles


def summarize(report):
    expected = 'stock framed management forwarder'
    if report['readinessTransport'] != expected or report['rssObservationTransport'] != expected:
        raise ValueError('Readiness and the memory observer must use the production management forwarder')
    samples = report['samples']
    if not report['complete'] or [row['sample'] for row in samples] != list(range(1, 16)):
        raise ValueError('The baseline requires all 15 accepted fresh boots')
    observations = []
    for row in samples:
        acceptance, obs = row['acceptance'], row['observation']
        if not all(acceptance[key] is True for key in
                   ['signedCleanShutdown', 'authenticatedInitShutdown', 'shutdownReplayRefused']):
            raise ValueError('Missing shutdown or replay proof')
        if obs['invalidBearerHealthRefused'] is not True:
            raise ValueError('Missing same-endpoint authentication check')
        plan, delivery, ready = [obs[key] for key in ['launchToMonitorPlanSentNs',
                                 'launchToBootstrapDeliveredNs', 'launchToAuthenticatedReadyNs']]
        forwarded = obs['launchToFramedForwarderNs']
        if not (0 < plan <= forwarded <= delivery <= ready
                and delivery - forwarded == obs['framedForwarderToBootstrapDeliveredNs']
                and ready - delivery == obs['bootstrapDeliveredToAuthenticatedReadyNs']):
            raise ValueError('Broken per-boot host clock ordering')
        if min(obs['readyToRssExecCompletedNs'], obs['guestInitRssKiB'], obs['guestServingAgentRssKiB']) <= 0:
            raise ValueError('Incomplete RSS probe')
        observations.append(obs)
    fields = [
        ('launchToMonitorPlanSentNs', 'Monitor plan sent', 1e6, 'ms'),
        ('launchToFramedForwarderNs', 'Framed host forwarder listening', 1e6, 'ms'),
        ('framedForwarderToBootstrapDeliveredNs', 'Framed forwarder to bootstrap delivery', 1e6, 'ms'),
        ('launchToBootstrapDeliveredNs', 'Bootstrap delivered', 1e6, 'ms'),
        ('launchToAuthenticatedReadyNs', 'Authenticated health', 1e6, 'ms'),
        ('bootstrapDeliveredToAuthenticatedReadyNs', 'Bootstrap delivery to health', 1e6, 'ms'),
        ('readyToRssExecCompletedNs', 'Health to RSS exec completion', 1e6, 'ms'),
        ('guestInitRssKiB', 'Guest PID 1 RSS', 1, 'KiB'),
        ('guestServingAgentRssKiB', 'Guest serving agent RSS', 1, 'KiB'),
    ]
    result = ['| Observation | p50 | p95 | Unit |', '| --- | ---: | ---: | --- |']
    for key, label, divisor, unit in fields:
        median, tail = percentiles([obs[key] for obs in observations])
        result.append(f'| {label} | {median / divisor:.3f} | {tail / divisor:.3f} | {unit} |')
    combined, combined_tail = percentiles([obs['guestInitRssKiB'] + obs['guestServingAgentRssKiB']
                                           for obs in observations])
    result.append(f'| Combined process RSS | {combined:.3f} | {combined_tail:.3f} | KiB |')
    delivery, _ = percentiles([obs['launchToBootstrapDeliveredNs'] for obs in observations])
    ready, _ = percentiles([obs['launchToAuthenticatedReadyNs'] for obs in observations])
    result += ['', f'With each observed bootstrap-delivery time held fixed, eliminating all remaining work '
               f'could improve median authenticated readiness by at most {100 * (1 - delivery / ready):.2f}%.',
               '', 'All health and RSS requests passed through the stock framed management forwarder. Combined process RSS '
               'sums the two observations within each boot. Both processes can map '
               'the same pages. Each row retains its own bootstrap-to-health interval and probe delay.']
    return '\n'.join(result) + '\n'


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('samples', type=Path)
    args = parser.parse_args()
    print(summarize(json.loads(args.samples.read_text())), end='')
