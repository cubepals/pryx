# SPDX-License-Identifier: FSL-1.1-ALv2
"""The README workflow against the CI one-box control and real daemon."""
import os, unittest, uuid
from cubepals_pryx import Pryx
@unittest.skipUnless(os.getenv("PRYX_API_URL") and os.getenv("PRYX_API_KEY"), "real control credentials not configured")
class EndToEnd(unittest.TestCase):
    def test_make_exec_snapshot_fork_delete(self):
        with Pryx(os.environ["PRYX_API_URL"],os.environ["PRYX_API_KEY"]) as client:
            sandbox=client.make({"image":"alpine:3.22","entrypoint":["sh","-c","trap 'exit 0' TERM; while :; do sleep 1; done"]},str(uuid.uuid4()))
            try:
                answer=list(client.exec(sandbox.id,["sh","-c","echo hello > /data/message; cat /data/message"],key=str(uuid.uuid4())))
                self.assertIn({"type":"stdout","data":"hello\n","truncated":False},answer)
                snap=client.snapshot(sandbox.id)
                fork=client.fork(snap.id)
                try: self.assertEqual(client.get(fork.id).state,"running")
                finally: client.delete(fork.id)
                client.action(sandbox.id,"sleep");client.action(sandbox.id,"wake")
            finally: client.delete(sandbox.id)
