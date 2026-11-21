#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Validate complete native cohorts and compare p50/p95 with their same-image stock arm."""
import argparse
import json
import math
from pathlib import Path
import statistics


def quantiles(values):
    data = sorted(values)
    return {'p50': statistics.median(data), 'p95': data[math.ceil(.95 * len(data)) - 1]}


def summarize(report):
    if report['complete'] is not True or report['readinessTransport'] != 'stock framed management forwarder':
        raise ValueError('Incomplete or incorrect authenticated endpoint')
    keys = ['stock', 'eager', 'activated']
    images = {'stock': ('/init', 'init'), 'eager': ('/lab-control', 'init'),
              'activated': ('/lab-init', 'lab-service')}
    if report['ordering'] != {'oddRounds':keys,'evenRounds':keys[::-1],'forwardRounds':8,'reverseRounds':7,'positionBase':1}:
        raise ValueError('Missing registered paired order')
    if len(report['samples']) != 45 or len(report['firstRequests']) != 2:
        raise ValueError('Expected fifteen complete stock/eager/activated triples and two other boots')
    for i, row in enumerate(report['firstRequests']):
        mode = ['raw', 'framed'][i]
        proof = row['observation']
        if (row['case'] != f'first-{mode}' or proof['mode'] != mode or
                proof['firstStatus'] != 200 or proof['secondStatus'] != 401 or
                proof['identityVerified'] is not True or proof['servingPid'] <= 1):
            raise ValueError('Either original management connection failed its signed functional checks')
    groups = {case: [] for case in keys}
    for n, row in enumerate(report['samples']):
        pair = 1 + n // 3
        order = keys if pair % 2 else keys[::-1]
        case = order[n % 3]
        if (row['pair'] != pair or row['case'] != case or row['position'] != 1 + n % 3 or
                row['direction'] != ('forward' if pair % 2 else 'reverse')):
            raise ValueError('Missing paired sample or incorrect whole-triple reversal')
        obs = row['observation']
        if (obs['pid1Executable'], obs['servingExecutable']) != images[case]:
            raise ValueError('A measured executable differs from its assigned role')
        if not (0 < obs['launchToMonitorPlanSentNs'] <= obs['launchToFramedForwarderNs'] <=
                obs['launchToBootstrapDeliveredNs'] <= obs['launchToAuthenticatedReadyNs']):
            raise ValueError('Boot phases are out of order')
        init, service = obs['guestInitRssKiB'], obs['guestServingAgentRssKiB']
        if min(init, service) <= 0 or obs['invalidBearerHealthRefused'] is not True:
            raise ValueError('Missing RSS or refusal')
        groups[case].append((obs['launchToAuthenticatedReadyNs'], init, service, init + service))
    for row in [*report['firstRequests'], *report['samples']]:
        if not all(row['acceptance'][k] is True for k in [
                'authenticatedInitShutdown', 'signedCleanShutdown', 'shutdownReplayRefused']):
            raise ValueError('Missing authenticated shutdown proof')
    names = ['readyNs', 'pid1RssKiB', 'servingRssKiB', 'withinBootSummedProcessRssKiB']
    stats = {case: {key: quantiles([sample[i] for sample in rows]) for i, key in enumerate(names)}
             for case, rows in groups.items()}
    baseline = stats['stock']
    for case in ['eager', 'activated']:
        current = stats[case]
        limits = {key: (1 - current[key]['p50'] / baseline[key]['p50']) for key in names}
        paired = {key: quantiles([s[i] - c[i] for s, c in zip(groups['stock'], groups[case])])
                  for i, key in enumerate(names)}
        current['medianReductionFraction'] = limits
        current['pairedStockMinusCandidate'] = paired
        current['readyCriterion'] = current['readyNs']['p50'] <= .90 * baseline['readyNs']['p50'] and current['readyNs']['p95'] <= 1.05 * baseline['readyNs']['p95']
        current['rssCriteriaByRole'] = {key: current[key]['p50'] <= .80 * baseline[key]['p50'] for key in names[1:]}
    return {'boots': 45, 'functionalBoots': 2, 'imageSha256': report['bootstrap']['imageSha256'], 'cohorts': stats,
            'rssInterpretation': 'Within-boot sum of two process VmRSS samples can count shared pages twice'}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('report', type=Path)
    args = parser.parse_args()
    print(json.dumps(summarize(json.loads(args.report.read_text())), indent=2))
