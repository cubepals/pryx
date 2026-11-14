# SPDX-License-Identifier: FSL-1.1-ALv2
"""A real pinned Debian DSC must refuse an export missing one of its original archives."""
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import desktop_sources


@unittest.skipUnless(os.environ.get('PRYX_DESKTOP_TEST_SOURCE_CACHE'), 'explicit complete Debian source cache')
class CompleteDesktopSources(unittest.TestCase):
    def test_a_real_dsc_cannot_export_without_its_source_archive(self):
        catalog = json.loads((desktop_sources.ROOT / 'desktop-sources.lock.json').read_text())
        archive = next(p for p in catalog if not p['url'].endswith('.dsc'))
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / 'desktop.lock.json').write_bytes((desktop_sources.ROOT / 'desktop.lock.json').read_bytes())
            (root / 'desktop-sources.lock.json').write_text(json.dumps([p for p in catalog if p != archive]))
            output = root / 'export'
            with patch.object(desktop_sources, 'ROOT', root):
                with self.assertRaisesRegex(ValueError, 'DSC source checksum'):
                    desktop_sources.collect(Path(os.environ['PRYX_DESKTOP_TEST_SOURCE_CACHE']), output)
            self.assertFalse(output.exists())


if __name__ == '__main__':
    unittest.main()
