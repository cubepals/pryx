# SPDX-License-Identifier: FSL-1.1-ALv2
"""Both Python handles against the real CI agent deployment and edge, including live streams.
This module is invoked explicitly by the container suite, never as a substitute local runtime.
"""
import asyncio
import os
import unittest
from urllib.parse import urlparse
import httpx
from cubepals_pryx import Pryx, AsyncPryx, PryxError

class AgentSync(unittest.TestCase):
    def test_complete_surface(self):
        with Pryx(os.environ['PRYX_API_URL'], os.environ['PRYX_API_KEY']) as sdk:
            box = sdk.sandbox(image='alpine:pryx-tls', sparePorts=1, resources={'memoryMb':256})
            try:
                process = box.start('printf first; read value; printf "%s" "$value"', stdin=True, cwd='/data', env={'CI':'1'})
                output = process.stream()
                self.assertEqual(next(output)['data'], 'first')
                self.assertTrue(box.agent.process(process.id).view['running'])
                process.stdin.write('last\n')
                self.assertEqual(next(output)['data'], 'last')
                list(output)
                self.assertEqual(process.wait().code, 0)
                self.assertEqual(box.run('printf hello').stdout, 'hello')
                for signal in ('TERM', 'KILL'):
                    background = box.start('sleep 60')
                    background.kill() if signal == 'KILL' else background.signal(signal)
                    self.assertNotEqual(background.wait().code, 0)
                terminal = box.terminal(cols=120, rows=40)
                terminal.resize(80, 24)
                terminal.stdin.write("printf 'pty:%s\\n' ready\n")
                received = ''
                for chunk in terminal.stream():
                    received += chunk['data']
                    if 'pty:ready' in received: break
                terminal.close()
                self.assertIn('pty:ready', received)
                box.files.mkdir('src')
                watch = box.files.watch('src')
                self.assertEqual(next(watch)['type'], 'ready')
                box.write('src/message', 'hello')
                self.assertEqual(next(watch)['type'], 'change')
                watch.close()
                box.files.move('src/message', 'src/moved')
                self.assertEqual(box.files.stat('src/moved')['size'], 5)
                self.assertTrue(any(f['name']=='moved' for f in box.files.list('src')))
                grant = box.files.url('src/moved')
                self.assertEqual(httpx.get(grant['url']).text, 'hello')
                data = b'x'*(3*1024*1024)
                box.write('large', iter([data[:10],data[10:]]))
                self.assertEqual(b''.join(box.read_stream('large')), data)
                box.write('index.html', 'exposed')
                web = box.start(['httpd','-f','-p','3000','-h','/data'])
                url = box.expose(3000)
                self.assertEqual(httpx.get(os.environ['PRYX_TEST_EDGE_URL'], headers={'host':urlparse(url).hostname}).text, 'exposed')
                with self.assertRaises(PryxError) as error: box.expose(3001)
                self.assertEqual(error.exception.code, 'no_spare_port')
                box.unexpose(3000); web.kill(); web.wait()
                box.files.delete('src/moved')
                box.sleep()
                self.assertEqual(box.run('printf awake').stdout, 'awake')
                box.stop(); box.write('stopped', data); self.assertEqual(box.read_bytes('stopped'), data)
                box.start()
                box.network('none'); self.assertEqual(box.run('printf isolated').stdout, 'isolated')
                box.network('internet'); self.assertEqual(box.run('printf online').stdout, 'online')
            finally: box.delete()

class AgentAsync(unittest.IsolatedAsyncioTestCase):
    async def test_complete_surface(self):
        async with AsyncPryx(os.environ['PRYX_API_URL'], os.environ['PRYX_API_KEY']) as sdk:
            box = await sdk.sandbox(image='alpine:pryx-tls', sparePorts=1, resources={'memoryMb':256})
            try:
                process = await box.start('printf first; read value; printf "%s" "$value"', stdin=True, cwd='/data', env={'CI':'1'})
                output = process.stream()
                self.assertEqual((await anext(output))['data'], 'first')
                self.assertTrue((await box.agent.process(process.id)).view['running'])
                await process.stdin.write('last\n')
                self.assertEqual((await anext(output))['data'], 'last')
                async for _ in output: pass
                self.assertEqual((await process.wait()).code, 0)
                self.assertEqual((await box.run('printf hello')).stdout, 'hello')
                for signal in ('TERM', 'KILL'):
                    background = await box.start('sleep 60')
                    await background.kill() if signal == 'KILL' else await background.signal(signal)
                    self.assertNotEqual((await background.wait()).code, 0)
                terminal = await box.terminal(cols=120, rows=40)
                await terminal.resize(80,24)
                await terminal.stdin.write("printf 'pty:%s\\n' ready\n")
                received = ''
                async for chunk in terminal.stream():
                    received += chunk['data']
                    if 'pty:ready' in received: break
                await terminal.close()
                self.assertIn('pty:ready', received)
                await box.files.mkdir('src')
                watch = box.files.watch('src')
                self.assertEqual((await anext(watch))['type'], 'ready')
                await box.write('src/message','hello')
                self.assertEqual((await anext(watch))['type'], 'change')
                await watch.aclose()
                await box.files.move('src/message','src/moved')
                self.assertEqual((await box.files.stat('src/moved'))['size'],5)
                self.assertTrue(any(f['name']=='moved' for f in await box.files.list('src')))
                grant = await box.files.url('src/moved')
                async with httpx.AsyncClient() as http: self.assertEqual((await http.get(grant['url'])).text,'hello')
                data = b'x'*(3*1024*1024)
                async def source():
                    yield data[:10]; yield data[10:]
                await box.write('large',source())
                self.assertEqual(b''.join([chunk async for chunk in box.read_stream('large')]),data)
                await box.write('index.html','exposed')
                web = await box.start(['httpd','-f','-p','3000','-h','/data'])
                url = await box.expose(3000)
                async with httpx.AsyncClient() as http: self.assertEqual((await http.get(os.environ['PRYX_TEST_EDGE_URL'],headers={'host':urlparse(url).hostname})).text,'exposed')
                with self.assertRaises(PryxError) as error: await box.expose(3001)
                self.assertEqual(error.exception.code,'no_spare_port')
                await box.unexpose(3000); await web.kill(); await web.wait()
                await box.files.delete('src/moved')
                await box.sleep(); self.assertEqual((await box.run('printf awake')).stdout,'awake')
                await box.stop(); await box.write('stopped',data); self.assertEqual(await box.read_bytes('stopped'),data)
                await box.start()
                await box.network('none'); self.assertEqual((await box.run('printf isolated')).stdout,'isolated')
                await box.network('internet'); self.assertEqual((await box.run('printf online')).stdout,'online')
            finally: await box.delete()

if __name__ == '__main__': unittest.main()
