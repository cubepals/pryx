# SPDX-License-Identifier: FSL-1.1-ALv2
"""Check owned Linux process-tree measurements and the opt-in socket response."""
import importlib.util
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

REPOSITORY = Path(__file__).resolve().parents[2]
SAMPLER = REPOSITORY / 'control/tools/tier-memory.py'


def load(path):
    spec = importlib.util.spec_from_file_location(path.stem, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


sampler = load(SAMPLER)
fixture = load(REPOSITORY / 'deploy/vm-memory-fixture.py')


class TierMemoryTest(unittest.TestCase):
    def test_helper_roles_measure_owned_executables_only(self):
        server = '''
import subprocess, sys
children = [subprocess.Popen([binary, '60']) for binary in sys.argv[1:]]
try:
    print('ready', flush=True)
    sys.stdin.read()
finally:
    for child in children:
        child.terminate()
        child.wait()
'''
        with tempfile.TemporaryDirectory() as folder:
            bins = [Path(folder) / name for name in ('firecracker', 'network-proxy', 'vm-transport')]
            for binary in bins:
                shutil.copy2(shutil.which('sleep'), binary)
            with subprocess.Popen([str(bins[2]), '60']) as outsider:
                try:
                    with subprocess.Popen([sys.executable, '-c', server, *bins],
                                          stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True) as owned:
                        try:
                            self.assertEqual(owned.stdout.readline(), 'ready\n')
                            result = sampler.sample({owned.pid}, vm_helpers=True)
                        finally:
                            owned.stdin.close()
                finally:
                    outsider.terminate()
        self.assertEqual(result['roots'], [owned.pid])
        self.assertEqual(result['processes'], 4)
        self.assertEqual(result['missing'], [])
        roles = result['vmHelpers']
        self.assertEqual(roles['launcher'], {'processes': 0, 'pssBytes': None})
        for role in ('firecracker', 'network-proxy', 'vm-transport'):
            self.assertEqual(roles[role]['processes'], 1)
            self.assertGreater(roles[role]['pssBytes'], 0)
        self.assertLess(sum(role['pssBytes'] or 0 for role in roles.values()), result['pssBytes'])

    def test_disappeared_process_keeps_missing_and_null_measurements(self):
        with subprocess.Popen([sys.executable, '-c', 'pass']) as child:
            child.wait()
        result = sampler.sample({child.pid}, vm_helpers=True)
        self.assertEqual(result['missing'], [child.pid])
        self.assertEqual(result['pssBytes'], 0)
        self.assertTrue(all(row == {'processes': 0, 'pssBytes': None} for row in result['vmHelpers'].values()))

    def test_exec_transition_keeps_total_without_assigning_pss_to_either_role(self):
        with patch.object(sampler, 'executable_role', side_effect=['launcher', 'firecracker']):
            result = sampler.sample({os.getpid()}, vm_helpers=True)
        self.assertGreater(result['pssBytes'], 0)
        self.assertEqual(result['vmHelpers']['launcher'], {'processes': 0, 'pssBytes': None})
        self.assertEqual(result['vmHelpers']['firecracker'], {'processes': 0, 'pssBytes': None})

    def test_socket_breakdown_requires_a_boolean_opt_in(self):
        def request(payload):
            client, server = socket.socketpair()
            with client, server:
                client.sendall(json.dumps({'root': os.getpid(), **payload}).encode())
                client.shutdown(socket.SHUT_WR)
                return fixture.respond(server, os.getuid(), SAMPLER)

        legacy = {'pssBytes', 'processes'}
        self.assertEqual(set(request({})), legacy)
        self.assertEqual(set(request({'vmHelpers': False})), legacy)
        measured = request({'vmHelpers': True})
        self.assertEqual(set(measured), legacy | {'vmHelpers'})
        self.assertEqual(set(measured['vmHelpers']), set(sampler.VM_EXECUTABLES.values()))
        with self.assertRaises(ValueError):
            request({'vmHelpers': 1})


if __name__ == '__main__':
    unittest.main()
