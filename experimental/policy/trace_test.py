# SPDX-License-Identifier: FSL-1.1-ALv2
"""Check relocation and trace ordering without loading a program."""
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch as mock_patch

from counted_object import INSN, cfg, executed_path, patch, path_counts
from run_coverage import run_image


class TraceTest(unittest.TestCase):
    def test_diamond_preserves_the_original_instructions_and_branch(self):
        original = [ # Conditional reaches the exit directly or via the notice block.
            (0xbf,0x16,0,0),(0x15,0x01,1,9),(0xb7,0,0,1),(0x95,0,0,0)]
        code = b"".join(INSN.pack(*i) for i in original)
        out, kept, targets, blocks = patch(code,48,1,107)
        for pos, wanted in enumerate(original):
            actual = INSN.unpack_from(out,8*kept[pos])
            if pos == 1:
                self.assertEqual(kept[pos]+1+actual[2],targets[3])
                self.assertEqual(actual[:2]+actual[3:],wanted[:2]+wanted[3:])
            else:
                self.assertEqual(actual,wanted)
        self.assertEqual([b['entry'] for b in executed_path(blocks,0)], [0,3])
        self.assertEqual([b['entry'] for b in executed_path(blocks,1)], [0,2,3])

    def test_verifier_failure_releases_all_owned_pins(self):
        original_directory = tempfile.TemporaryDirectory
        with original_directory() as scratch:
            root = Path(scratch)
            output = root / "evidence"
            output.mkdir()
            def private_dir(prefix, dir):
                self.assertEqual(dir, "/sys/fs/bpf")
                return original_directory(prefix=prefix,dir=root)
            failure = subprocess.CalledProcessError(1, ["bpftool", "prog", "loadall"])
            with mock_patch("run_coverage.tempfile.TemporaryDirectory", side_effect=private_dir), \
                 mock_patch("run_coverage.subprocess.run", side_effect=failure):
                with self.assertRaises(subprocess.CalledProcessError):
                    run_image("bpftool", root/"fixture.o",output,[192,48,20,1,107])
            self.assertEqual(set(root.iterdir()),{output})
            self.assertFalse((output/"instruction-counts.json").exists())

    def test_single_run_can_distinguish_more_than_a_word_of_paths(self):
        # Each taken edge skips one harmless assignment. Forty diamonds have 2^40 paths.
        pair = INSN.pack(0x15,1,1,0) + INSN.pack(0xb7,0,0,1)
        code = pair*40 + INSN.pack(0x95,0,0,0)
        out,_,_,blocks = patch(code,48,1,107)
        self.assertEqual(path_counts(blocks)[0],1<<40)
        self.assertTrue(any(out[i] == 0x18 for i in range(0,len(out),8)))
        self.assertEqual(len([p for b in executed_path(blocks,0) for p in b["pcs"]]),41)
        self.assertEqual(len([p for b in executed_path(blocks,(1<<40)-1) for p in b["pcs"]]),81)
        with self.assertRaises(ValueError):
            executed_path(blocks,1<<40)

    def test_reject_cycle(self):
        with self.assertRaises(ValueError):
            cfg(INSN.pack(5,0,-1,0))


if __name__ == "__main__":
    unittest.main()
