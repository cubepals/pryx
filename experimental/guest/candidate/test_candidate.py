# SPDX-License-Identifier: FSL-1.1-ALv2
"""Check the isolated archive's three-path boundary and the native comparison reader."""
import copy
import hashlib
import json
from pathlib import Path
import tempfile
import unittest

import bundle
import evidence


def measured():
    accept = dict.fromkeys(['authenticatedInitShutdown', 'signedCleanShutdown', 'shutdownReplayRefused'], True)
    def row(case, pair, observation):
        return {'case': case, 'pair': pair, 'observation': observation, 'acceptance': accept}
    first = [row(f'first-{mode}', 0, {'mode': mode, 'firstStatus': 200, 'secondStatus': 401,
                    'identityVerified': True, 'servingPid': 11}) for mode in ['raw','framed']]
    cases = [('stock','/init','init'), ('eager','/lab-control','init'), ('activated','/lab-init','lab-service')]
    samples = []
    for i in range(15):
        for pos, (case, init, service) in enumerate(cases if i % 2 == 0 else cases[::-1], start=1):
            samples.append(row(case, i + 1, {'pid1Executable': init, 'servingExecutable': service,
                           'launchToMonitorPlanSentNs': 3, 'launchToFramedForwarderNs': 4,
                           'launchToBootstrapDeliveredNs': 100, 'launchToAuthenticatedReadyNs': 200,
                           'guestInitRssKiB': 50, 'guestServingAgentRssKiB': 80, 'invalidBearerHealthRefused': True}))
            samples[-1].update(position=pos,direction='forward' if i % 2 == 0 else 'reverse')
    return {'ordering':{'oddRounds':['stock','eager','activated'],'evenRounds':['activated','eager','stock'],
                        'forwardRounds':8,'reverseRounds':7,'positionBase':1},
            'samples': samples, 'firstRequests': first, 'bootstrap': {'imageSha256':'synthetic'},
            'complete': True, 'readinessTransport': 'stock framed management forwarder'}


class Candidate(unittest.TestCase):
    def test_positive_and_rejected_measurements(self):
        source = measured()
        got = evidence.summarize(source)
        self.assertEqual(got['cohorts']['activated']['withinBootSummedProcessRssKiB']['p50'], 130)
        self.assertFalse(got['cohorts']['activated']['readyCriterion'])
        for group, index, key, value in [
                ('samples', 3, 'pair', 7), ('samples', 3, 'direction', 'forward'), ('samples', 8, 'position', 1), ('samples', 1, 'observation', {}), ('firstRequests', 0, 'observation', {})]:
            bad = copy.deepcopy(source)
            bad[group][index][key] = value
            with self.assertRaises((ValueError, KeyError)):
                evidence.summarize(bad)
        short = copy.deepcopy(source)
        short['samples'].pop()
        with self.assertRaises(ValueError):
            evidence.summarize(short)

    def test_private_inventory_replacement(self):
        with tempfile.TemporaryDirectory() as t:
            root = Path(t)
            stock, staged, output = root/'stock', root/'staged', root/'out'
            (stock/'bin').mkdir(parents=True)
            (stock/'bootstrap').mkdir()
            staged.mkdir()
            (stock/'bin/vm-launcher').write_bytes(b'original host agent')
            (stock/'bin/vm-config').write_bytes(b'original configurator')
            config = root/'candidate-config'
            config.write_bytes(b'tested candidate configurator')
            old = stock/'bootstrap/bootstrap.erofs'
            old.write_bytes(b'original image')
            new = staged/'bootstrap.erofs'
            new.write_bytes(b'new image')
            digest = lambda path: hashlib.sha256(path.read_bytes()).hexdigest()
            original = {'agentSha256': digest(stock/'bin/vm-launcher'), 'filesystemManifestSha256': 'fs',
                        'networkHelperSha256':'helper', 'packetForwarderSha256':'packet', 'metadata': {},
                        'networkProbe':None, 'imageSha256':digest(old)}
            (stock/'bootstrap/bootstrap.json').write_text(json.dumps(original))
            bundle.stock.inventory(stock)
            previous = digest(stock/'manifest.json')
            candidate = dict(original, imageSha256=digest(new))
            (staged/'bootstrap.json').write_text(json.dumps(candidate))
            bundle.build(stock, staged, config, output)
            descriptor = json.loads((output/'descriptor.json').read_text())
            self.assertEqual(descriptor['sha256'], digest(output/'runtime.tar.gz'))
            self.assertEqual(previous, digest(stock/'manifest.json'))
            self.assertEqual(bundle.stock.package.verify(stock), None)
            candidate['agentSha256'] = 'different'
            (staged/'bootstrap.json').write_text(json.dumps(candidate))
            with self.assertRaises(ValueError):
                bundle.build(stock, staged, config, root/'untrusted')


if __name__ == '__main__':
    unittest.main()
