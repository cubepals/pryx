# SPDX-License-Identifier: FSL-1.1-ALv2
"""Real HTTP process streams finish at the final exit frame and release their connection.
The peer withholds transport EOF so process completion cannot accidentally depend on it.
"""
import asyncio
import base64
import json
import socket
import threading
import unittest
from concurrent.futures import Future
from http import HTTPStatus
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from types import SimpleNamespace
import httpx
from cubepals_pryx import PryxError
from cubepals_pryx.agent import Process
from cubepals_pryx.async_agent import AsyncProcess
from cubepals_pryx.run import CommandStream, AsyncCommandStream

BOUND = 3
RESULT = dict(code=0, timedOut=False, killed=False, durationMs=1)
OUTPUT = dict(type='output', stream='stdout', bytes=base64.b64encode(b'hello').decode())
EXIT = dict(type='exit', result=RESULT)
PARTIAL = {**OUTPUT, 'bytes': base64.b64encode(b'\xc3').decode()}

class OutputPeer:
    def __init__(self, frames, hold=True):
        self.sent, self.disconnected = threading.Event(), threading.Event()
        self.connection = None
        owner = self
        class Handler(BaseHTTPRequestHandler):
            protocol_version = 'HTTP/1.1'
            def log_message(self, *_): pass
            def do_GET(self):
                owner.connection = self.connection
                self.send_response(HTTPStatus.OK)
                self.send_header('Content-Type', 'application/x-ndjson')
                self.send_header('Transfer-Encoding', 'chunked')
                self.end_headers()
                body = b''.join((json.dumps(frame)+'\n').encode() for frame in frames)
                self.wfile.write(f'{len(body):x}\r\n'.encode()+body+b'\r\n')
                if not hold: self.wfile.write(b'0\r\n\r\n')
                self.wfile.flush()
                owner.sent.set()
                if hold:
                    try:
                        self.connection.settimeout(BOUND * 2)
                        if self.connection.recv(1) == b'': owner.disconnected.set()
                    except OSError: pass
                self.close_connection = True
        self.server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        self.thread = threading.Thread(target=self.server.serve_forever)
        self.thread.start()
        self.url = f'http://127.0.0.1:{self.server.server_port}'
    def __enter__(self): return self
    def disconnect(self):
        if self.connection:
            try: self.connection.shutdown(socket.SHUT_RDWR)
            except OSError: pass
    def __exit__(self, *_):
        self.disconnect()
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(BOUND)
    def process(self, http):
        agent = SimpleNamespace(http=http, target=lambda path: (self.url+path, {}))
        return Process(agent, {'id': 'owned-process'})
    def async_process(self, http):
        async def target(path): return self.url+path, {}
        return AsyncProcess(SimpleNamespace(http=http, target=target), {'id': 'owned-process'})

def in_thread(operation):
    result = Future()
    def run():
        try: result.set_result(operation())
        except BaseException as error: result.set_exception(error)
    thread = threading.Thread(target=run)
    thread.start()
    return result, thread

class SyncCompletion(unittest.TestCase):
    def test_final_event_closes_before_caller_resumes(self):
        with OutputPeer([OUTPUT, EXIT]) as peer, httpx.Client(timeout=None, trust_env=False) as http:
            events = peer.process(http).events()
            try:
                self.assertEqual(next(events), OUTPUT)
                self.assertEqual(next(events), EXIT)
                self.assertTrue(peer.disconnected.wait(BOUND), 'final event retained HTTP connection')
            finally: events.close()
    def test_stream_and_result_finish_without_transport_eof(self):
        for operation in ('stream', 'result'):
            with self.subTest(operation=operation), OutputPeer([OUTPUT, PARTIAL, EXIT]) as peer:
                with httpx.Client(timeout=None, trust_env=False) as http:
                    process = peer.process(http)
                    future, worker = in_thread(lambda: list(process.stream()) if operation == 'stream' else CommandStream(process).result())
                    try:
                        self.assertTrue(peer.sent.wait(BOUND))
                        result = future.result(BOUND)
                        if operation == 'stream': self.assertEqual(result, [{'stream': 'stdout', 'data': 'hello'}, {'stream': 'stdout', 'data': '\ufffd'}])
                        else: self.assertEqual((result.stdout, result.code), ('hello\ufffd', RESULT['code']))
                        self.assertTrue(peer.disconnected.wait(BOUND))
                    finally:
                        peer.disconnect()
                        worker.join(BOUND)
    def test_missing_exit_and_invalid_frames_remain_errors(self):
        for frames, error_type in (([OUTPUT], PryxError), ([{'type': 'output', 'stream': 'stdout', 'bytes': '!'}], ValueError), ([{'type': 'exit'}], KeyError)):
            with self.subTest(frames=frames), OutputPeer(frames, hold=False) as peer:
                with httpx.Client(timeout=BOUND, trust_env=False) as http:
                    with self.assertRaises(error_type) as caught: CommandStream(peer.process(http)).result()
                    if error_type is PryxError: self.assertEqual(caught.exception.code, 'exec_interrupted')

class AsyncCompletion(unittest.IsolatedAsyncioTestCase):
    async def test_final_event_closes_before_caller_resumes(self):
        with OutputPeer([OUTPUT, EXIT]) as peer:
            async with httpx.AsyncClient(timeout=None, trust_env=False) as http:
                events = peer.async_process(http).events()
                try:
                    self.assertEqual(await anext(events), OUTPUT)
                    self.assertEqual(await anext(events), EXIT)
                    self.assertTrue(await asyncio.to_thread(peer.disconnected.wait, BOUND), 'final event retained HTTP connection')
                finally: await events.aclose()
    async def test_stream_and_result_finish_without_transport_eof(self):
        for operation in ('stream', 'result'):
            with self.subTest(operation=operation), OutputPeer([OUTPUT, PARTIAL, EXIT]) as peer:
                async with httpx.AsyncClient(timeout=None, trust_env=False) as http:
                    process = peer.async_process(http)
                    async def consume():
                        if operation == 'stream': return [chunk async for chunk in process.stream()]
                        async def started(): return process
                        return await AsyncCommandStream(started()).result()
                    result = await asyncio.wait_for(consume(), BOUND)
                    if operation == 'stream': self.assertEqual(result, [{'stream': 'stdout', 'data': 'hello'}, {'stream': 'stdout', 'data': '\ufffd'}])
                    else: self.assertEqual((result.stdout, result.code), ('hello\ufffd', RESULT['code']))
                    self.assertTrue(await asyncio.to_thread(peer.disconnected.wait, BOUND))
    async def test_missing_exit_and_invalid_frames_remain_errors(self):
        for frames, error_type in (([OUTPUT], PryxError), ([{'type': 'output', 'stream': 'stdout', 'bytes': '!'}], ValueError), ([{'type': 'exit'}], KeyError)):
            with self.subTest(frames=frames), OutputPeer(frames, hold=False) as peer:
                async with httpx.AsyncClient(timeout=BOUND, trust_env=False) as http:
                    async def started(): return peer.async_process(http)
                    with self.assertRaises(error_type) as caught: await AsyncCommandStream(started()).result()
                    if error_type is PryxError: self.assertEqual(caught.exception.code, 'exec_interrupted')
