# SPDX-License-Identifier: FSL-1.1-ALv2
"""The README workflow against the CI one-box control and real daemon."""
import os, unittest, uuid
from cubepals_pryx import Pryx
@unittest.skipUnless(os.getenv("PRYX_API_URL") and os.getenv("PRYX_API_KEY"), "real control credentials not configured")
class EndToEnd(unittest.TestCase):
    def test_make_exec_snapshot_fork_delete(self):
        with Pryx(url=os.environ["PRYX_API_URL"],key=os.environ["PRYX_API_KEY"]) as client:
            sandbox=client.sandbox(image="alpine:3.22",key=str(uuid.uuid4()))
            try:
                answer=sandbox.run("echo hello > /data/message; cat /data/message")
                self.assertEqual(answer.stdout,'hello\n')
                snap=sandbox.snapshot()
                fork=snap.fork()
                try: self.assertEqual(fork.refresh().state,"running")
                finally: fork.delete()
                sandbox.sleep();sandbox.wake()
            finally: sandbox.delete()
