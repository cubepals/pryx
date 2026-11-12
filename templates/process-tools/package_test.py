# SPDX-License-Identifier: FSL-1.1-ALv2
"""Real archive determinism, unchanged runtime payloads and inventory-tampering refusals."""
import json
import os
from pathlib import Path
import tarfile
import tempfile
import unittest
import package as packager


class PackageTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name) / 'bundle with spaces'
        self.root.mkdir()
        self.binary = self.root / 'bin' / 'runtime'
        self.binary.parent.mkdir()
        self.binary.write_bytes(b'unchanged upstream executable\x00\xff')
        self.binary.chmod(0o755)
        self.link = self.root / 'runtime'
        self.link.symlink_to('bin/runtime')
        self.long = self.root / ('a' * 90) / ('b' * 90) / ('c' * 90) / 'notice'
        self.long.parent.mkdir(parents=True)
        self.long.write_text('complete notice\n')
        entries = [dict(path=str(path.relative_to(self.root)), sha256=packager.digest(path))
                   for path in [self.binary, self.long]]
        entries.append(dict(path='runtime', link='bin/runtime'))
        (self.root / 'manifest.json').write_text(json.dumps(dict(format=1, files=entries)))
        self.archive = Path(self.temporary.name) / 'tools.tar.gz'

    def test_archive_is_deterministic_and_bounds_describe_real_members(self):
        before = self.binary.read_bytes()
        first = packager.package(self.root, self.archive, 'https://example.test/tools.tar.gz')
        with tarfile.open(self.archive) as archive:
            members = archive.getmembers()
            self.assertEqual(first['entries'], len(members))
            self.assertEqual(first['unpacked_bytes'], sum(item.size for item in members))
            self.assertEqual(archive.extractfile('bin/runtime').read(), before)
            self.assertTrue(archive.getmember('bin/runtime').mode & 0o111)
            self.assertEqual(archive.getmember('runtime').linkname, 'bin/runtime')
            self.assertTrue(all(item.uid == item.gid == item.mtime == 0 for item in members))
        os.utime(self.binary, (12345, 12345))
        second = packager.package(self.root, self.archive, first['url'])
        self.assertEqual(first, second)
        self.assertEqual(self.binary.read_bytes(), before)

    def test_changed_missing_surplus_and_escaping_files_are_refused(self):
        self.binary.write_bytes(b'changed')
        with self.assertRaisesRegex(ValueError, 'file changed'):
            packager.package(self.root, self.archive, 'https://example.test/tools.tar.gz')
        self.binary.unlink()
        with self.assertRaises(ValueError):
            packager.verify(self.root)
        self.binary.write_bytes(b'unchanged upstream executable\x00\xff')
        surplus = self.root / 'unlisted'
        surplus.write_text('unlisted')
        with self.assertRaisesRegex(ValueError, 'differs'):
            packager.verify(self.root)
        surplus.unlink()
        self.link.unlink()
        self.link.symlink_to(self.archive.parent)
        with self.assertRaisesRegex(ValueError, 'escapes'):
            packager.verify(self.root)


if __name__ == '__main__':
    unittest.main()
