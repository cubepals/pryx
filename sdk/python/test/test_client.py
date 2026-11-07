# SPDX-License-Identifier: FSL-1.1-ALv2
"""Generated request shapes, streaming and privacy without a real daemon."""
from http import HTTPStatus
import unittest
import httpx
from cubepals_pryx import Pryx, PryxError
from cubepals_pryx._transport import Transport

SANDBOX={"id":"sandbox","project":"default","state":"running","node":None,"epoch":None,"endpoints":[],"usage":None,"issues":[],"createdAt":"2026-11-04T00:00:00Z","updatedAt":"2026-11-04T00:00:00Z","expiresAt":None}
class ClientTests(unittest.TestCase):
    def test_generated_make_and_credential_privacy(self):
        def handler(request):
            self.assertEqual(request.headers["authorization"],"Bearer private-key")
            self.assertEqual(request.headers["idempotency-key"],"create-one")
            return httpx.Response(HTTPStatus.CREATED,json=SANDBOX)
        with Pryx(url="https://pryx.test",key="private-key",transport=httpx.MockTransport(handler)) as client:
            self.assertNotIn("private-key",repr(client))
            self.assertEqual(client.sandbox(image="alpine:3.22",key="create-one").id,"sandbox")
    def test_streamed_exec_events(self):
        def handler(request): return httpx.Response(HTTPStatus.OK,content='{"type":"stdout","data":"é"}\n{"type":"exit","code":0}\n',headers={"content-type":"application/x-ndjson"})
        transport=Transport('https://pryx.test','private-key',transport=httpx.MockTransport(handler))
        try:self.assertEqual(list(transport.lines('/v1/sandboxes/sandbox/logs'))[-1]['code'],0)
        finally:transport.close()
    def test_error_envelope_has_no_retry(self):
        calls=[]
        def handler(request): calls.append(request);return httpx.Response(HTTPStatus.SERVICE_UNAVAILABLE,json={"error":{"code":"no_room","message":"No room"}})
        with Pryx(url="https://pryx.test",key="private-key",transport=httpx.MockTransport(handler)) as client:
            with self.assertRaises(PryxError) as error: client.sandbox(image="alpine:3.22")
            self.assertEqual(error.exception.code,"no_room")
            self.assertEqual(len(calls),1)
