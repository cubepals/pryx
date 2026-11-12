# SPDX-License-Identifier: FSL-1.1-ALv2
"""Real complete desktop closure cannot borrow a host library or carry a private glibc."""
import os
import json
import shutil
from pathlib import Path
import re
import subprocess
import tempfile
import unittest
from archive_inputs import GLIBC_SONAMES
import desktop_full
import build


@unittest.skipUnless(os.environ.get('PRYX_DESKTOP_FULL_TEST_COMPONENT'), 'explicit complete desktop component')
class FullDesktopLibraryBoundary(unittest.TestCase):
    def setUp(self):
        self.root = Path(os.environ['PRYX_DESKTOP_FULL_TEST_COMPONENT']).resolve()

    def test_chromium_cannot_borrow_a_missing_own_library(self):
        binary = self.root / 'payload/usr/lib/chromium/chromium'
        dynamic = subprocess.check_output(['readelf', '--dynamic', str(binary)], text=True)
        needed = re.findall(r'\(NEEDED\).*\[([^]]+)\]', dynamic)
        name = next(name for name in needed if name.startswith('libpulse'))
        source = next(path / name for path in desktop_full.library_paths(self.root) if (path / name).is_file())
        with tempfile.TemporaryDirectory(prefix='pryx-full-desktop-library-') as directory:
            held = Path(directory) / name
            source.rename(held)
            try:
                with self.assertRaises(ValueError) as failure:
                    desktop_full.audit(self.root)
                self.assertIn(name, str(failure.exception))
            finally:
                held.rename(source)

    def test_private_glibc_is_refused_before_execution(self):
        marker = self.root / 'payload' / next(name for name in GLIBC_SONAMES if name.startswith('libc.'))
        self.assertFalse(marker.exists())
        marker.write_text('Forbidden private-libc test marker, never executed')
        try:
            with self.assertRaisesRegex(ValueError, 'private glibc'):
                desktop_full.audit(self.root)
        finally:
            marker.unlink()

    def test_actual_desktop_elf_cannot_raise_the_core_glibc_floor(self):
        minimum = tuple(map(int, json.loads((build.ROOT / 'compatibility.json').read_text())['hostGlibcMinimum'].split('.')))
        selected = None
        for item in desktop_full.inputs():
            for relative in item['elfs']:
                path = self.root / 'payload' / relative
                versions = subprocess.check_output(['readelf', '--version-info', '--wide', str(path)], text=True, stderr=subprocess.DEVNULL)
                required = [tuple(map(int, v.split('.'))) for v in re.findall(r'GLIBC_(\d+\.\d+(?:\.\d+)?)', versions)]
                if required and max(required) > minimum:
                    selected = path
                    break
            if selected is not None:
                break
        self.assertIsNotNone(selected, 'Use a real upstream ELF with the desktop-only host requirement')
        with tempfile.TemporaryDirectory(prefix='pryx-glibc-groups-') as directory:
            root = Path(directory)
            (root / 'desktop').mkdir()
            target = root / 'desktop' / selected.name
            shutil.copy2(selected, target)
            build.inventory(root)
            manifest = json.loads((root / 'manifest.json').read_text())
            self.assertEqual(manifest['hostGlibcMinimum'], '.'.join(map(str, minimum)))
            self.assertGreater(tuple(map(int, manifest['desktopMaximumRequiredGlibc'].split('.'))), minimum)
            target.rename(root / target.name)
            with self.assertRaisesRegex(ValueError, 'Bundle ELF exceeds'):
                build.inventory(root)


if __name__ == '__main__':
    unittest.main()
