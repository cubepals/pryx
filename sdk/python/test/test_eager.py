# SPDX-License-Identifier: FSL-1.1-ALv2
"""Actual command side effects prove eager dispatch without awaiting the native task."""
import asyncio
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
import httpx
from cubepals_pryx import Pryx
from test_agent import Peer

class Eager(unittest.IsolatedAsyncioTestCase):
    async def test_run_and_stream_start_without_await(self):
        for method in ('run','stream'):
            with tempfile.TemporaryDirectory() as folder:
                peer=Peer()
                def dispatch(request):
                    if request.url.path=='/_pryx/processes':
                        command=json.loads(request.content)['command']
                        subprocess.run(['sh','-c',command],cwd=folder,check=True)
                    return peer.handle(request)
                async with Pryx(url='http://control.test',key='secret',transport=httpx.MockTransport(dispatch)) as sdk:
                    box=await sdk.sandbox('one')
                    operation=getattr(box,method)('printf started > marker')
                    self.assertEqual(Path(folder,'marker').read_text(),'started')
                    if method=='run':
                        self.assertIsInstance(operation,asyncio.Task)
                        result=await operation
                    else:
                        self.assertFalse(hasattr(operation,'__await__'))
                        result=await operation.result()
                    self.assertEqual(result.code,0)
                    self.assertEqual(len([c for c in peer.calls if c[1]=='/_pryx/processes']),1)
