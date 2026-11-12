# SPDX-License-Identifier: FSL-1.1-ALv2
"""Real staged ELF dependency and host-libc boundary checks, run explicitly after CI assembly."""
import os
from pathlib import Path
import re
import subprocess
import tempfile
import unittest
import desktop


@unittest.skipUnless(os.environ.get('PRYX_DESKTOP_TEST_COMPONENT'), 'explicit assembled X11 component')
class DesktopLibraryBoundary(unittest.TestCase):
    def setUp(self):
        self.root = Path(os.environ['PRYX_DESKTOP_TEST_COMPONENT']).resolve()

    def test_a_host_copy_cannot_hide_a_missing_component_library(self):
        binary = self.root / 'payload/usr/bin/Xvfb'
        dynamic = subprocess.check_output(['readelf', '--dynamic', str(binary)], text=True)
        needed = re.findall(r'\(NEEDED\).*\[([^]]+)\]', dynamic)
        directory = self.root / 'payload/usr/lib/x86_64-linux-gnu'
        name = next(name for name in needed if name.startswith('libXfont') and (directory / name).is_file())
        source = directory / name
        with tempfile.TemporaryDirectory(prefix='pryx-desktop-owned-library-') as temporary:
            held = Path(temporary) / name
            source.rename(held)
            try:
                with self.assertRaises(ValueError) as failure:
                    desktop.audit(self.root)
                self.assertIn(name, str(failure.exception))
            finally:
                held.rename(source)

    def test_a_private_libc_marker_is_rejected_without_execution(self):
        marker = self.root / 'libraries/libc.so.6'
        self.assertFalse(marker.exists())
        marker.write_text('Forbidden test marker; not an executable or a real libc')
        try:
            with self.assertRaisesRegex(ValueError, 'private glibc'):
                desktop.audit(self.root)
        finally:
            marker.unlink()


if __name__ == '__main__':
    unittest.main()
