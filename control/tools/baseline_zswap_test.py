# SPDX-License-Identifier: FSL-1.1-ALv2
import importlib.util
import os
from pathlib import Path
import shutil
import tempfile
import unittest
from unittest.mock import patch


spec = importlib.util.spec_from_file_location('baseline_zswap', Path(__file__).with_name('baseline-zswap.py'))
probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(probe)


class BaselineZswap(unittest.TestCase):
    def test_ratio_uses_paired_private_counter_deltas(self):
        before = {'compressedBytes': 1024, 'logicalBytes': 8192}
        after = {'compressedBytes': 1024 + probe.MIB // 2, 'logicalBytes': 8192 + 2 * probe.MIB}
        self.assertEqual(probe.compression(before, after, True), {
            'newLogicalBytes': 2 * probe.MIB,
            'newCompressedBytes': probe.MIB // 2,
            'logicalToCompressedRatio': 4,
        })

    def test_no_ratio_when_counters_are_unstable_or_empty(self):
        before = {'compressedBytes': 0, 'logicalBytes': 0}
        for after, stable in [
            ({'compressedBytes': 200, 'logicalBytes': 2 * probe.MIB}, False),
            ({'compressedBytes': 0, 'logicalBytes': 2 * probe.MIB}, True),
            ({'compressedBytes': 40, 'logicalBytes': 4096}, True),
        ]:
            with self.subTest(after=after, stable=stable):
                result = probe.compression(before, after, stable)
                self.assertIn('reason', result)
                self.assertNotIn('logicalToCompressedRatio', result)

    @unittest.skipUnless(shutil.which('node'), 'Node is required for the loopback fixture')
    def test_actual_development_fixture_serves_and_retires(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = b'const value = 1\n'
            (root / 'example.ts').write_bytes(source)
            with (patch.object(probe, 'SOURCE_ROOT', root),
                  patch.object(probe, 'source_paths', return_value=['example.ts']),
                  patch.object(probe, 'joined')):
                with probe.development_server(root, shutil.which('node'), os.getuid(), os.getgid()) as (pid, port, note):
                    self.assertEqual(note['sourceFiles'], 1)
                    self.assertEqual(note['sourceBytes'], len(source))
                    self.assertEqual(note['sourceLines'], 2)
                    self.assertEqual(probe.request(port), note)
                with self.assertRaises(ProcessLookupError):
                    os.kill(pid, 0)


if __name__ == '__main__':
    unittest.main()
