# SPDX-License-Identifier: FSL-1.1-ALv2
"""Use both Python client modes against an actual installed VM selected by the fixture."""
import asyncio
import inspect
import os
from uuid import uuid4
from cubepals_pryx import Pryx


async def exercise(client, mode):
    async def take(value):
        return await value if inspect.isawaitable(value) else value
    box = await take(client.sandbox(os.environ['PRYX_VM_SANDBOX']))
    assert box.isolation == 'vm'
    proof = str(uuid4())
    path = 'python-' + mode
    await take(box.write(path, proof))
    result = await take(box.run(['/bin/sh', '-c', 'cat "$1"', 'vm-python', path]))
    assert result.code == 0 and result.stdout == proof
    assert await take(box.read(path)) == proof


with Pryx(url=os.environ['PRYX_API_URL'], key=os.environ['PRYX_API_KEY']) as client:
    asyncio.run(exercise(client, 'sync'))


async def main():
    async with Pryx(url=os.environ['PRYX_API_URL'], key=os.environ['PRYX_API_KEY']) as client:
        await exercise(client, 'async')


asyncio.run(main())
