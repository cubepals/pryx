# SPDX-License-Identifier: FSL-1.1-ALv2
"""Real one-box sync and async handles. Missing credentials are an error, never a passing skip."""
import os, unittest
from cubepals_pryx import Pryx
URL, KEY = os.environ['PRYX_API_URL'], os.environ['PRYX_API_KEY']
class SyncHandles(unittest.TestCase):
    def test_live_files_and_lifecycle(self):
        with Pryx(url=URL,key=KEY) as pryx:
            box = pryx.sandbox(image='alpine:3.22',idle='5m',ttl='2h')
            try:
                command = box.stream('echo ready; while [ ! -f /data/sync-go ]; do sleep 0.1; done; echo done',timeout='10s')
                lines = command.lines()
                self.assertEqual(next(lines),'ready')
                box.write('sync-go','go')
                self.assertEqual(list(lines),['done'])
                self.assertEqual(command.result().code,0); self.assertFalse(command.result().timed_out)
                data = os.urandom(180_000)
                box.write('nested/data',data); self.assertEqual(box.read_bytes('nested/data'),data)
                twin = box.snapshot().fork(idle='5m')
                try: self.assertEqual(twin.read_bytes('nested/data'),data)
                finally: twin.delete()
                box.sleep(); box.wake(); box.stop(); box.start(); box.rest(); box.wake()
                self.assertEqual(box.refresh().state,'running')
                self.assertIn(box.id,[sandbox.id for sandbox in pryx.sandboxes()])
            finally: box.delete()
class AsyncHandles(unittest.IsolatedAsyncioTestCase):
    async def test_live_files_and_fork(self):
        async with Pryx(url=URL,key=KEY) as pryx:
            box = await pryx.sandbox(image='alpine:3.22',idle='5m')
            try:
                command = box.stream('echo ready; while [ ! -f /data/async-go ]; do sleep 0.1; done; echo done',timeout='10s')
                lines = command.lines()
                self.assertEqual(await anext(lines),'ready')
                await box.write('async-go',b'go')
                self.assertEqual([line async for line in lines],['done'])
                result = await command.result()
                self.assertEqual(result.code,0); self.assertFalse(result.timed_out)
                data = os.urandom(100_000)
                await box.write('nested/data',data); self.assertEqual(await box.read_bytes('nested/data'),data)
                twin = await (await box.snapshot()).fork(idle='5m')
                try: self.assertEqual(await twin.read_bytes('nested/data'),data)
                finally: await twin.delete()
                await box.sleep(); await box.wake(); await box.stop(); await box.start(); await box.rest(); await box.wake()
                self.assertEqual((await box.refresh()).state,'running')
            finally: await box.delete()
if __name__ == '__main__': unittest.main(verbosity=2)
