# SPDX-License-Identifier: FSL-1.1-ALv2
"""Generated request shapes, streaming and privacy without a real daemon."""
from http import HTTPStatus
import unittest
import httpx
from cubepals_pryx import Pryx, PryxError

SANDBOX={"id":"sandbox","project":"default","state":"running","node":None,"epoch":None,"endpoints":[],"usage":None,"issues":[],"createdAt":"2026-11-04T00:00:00Z","updatedAt":"2026-11-04T00:00:00Z","expiresAt":None}
class ClientTests(unittest.TestCase):
    def test_generated_make_and_credential_privacy(self):
        def handler(request):
            self.assertEqual(request.headers["authorization"],"Bearer private-key")
            self.assertEqual(request.headers["idempotency-key"],"create-one")
            return httpx.Response(HTTPStatus.CREATED,json=SANDBOX)
        with Pryx("https://pryx.test","private-key",transport=httpx.MockTransport(handler)) as client:
            self.assertNotIn("private-key",repr(client._client))
            self.assertEqual(client.make({"image":"alpine:3.22"},"create-one").id,"sandbox")
    def test_streamed_exec_events(self):
        def handler(request): return httpx.Response(HTTPStatus.OK,content='{"type":"stdout","data":"é"}\n{"type":"exit","code":0}\n',headers={"content-type":"application/x-ndjson"})
        with Pryx("https://pryx.test","private-key",transport=httpx.MockTransport(handler)) as client:
            self.assertEqual(list(client.exec("sandbox",["echo","é"]))[-1]["code"],0)
    def test_error_envelope_has_no_retry(self):
        calls=[]
        def handler(request): calls.append(request);return httpx.Response(HTTPStatus.SERVICE_UNAVAILABLE,json={"error":{"code":"no_room","message":"No room"}})
        with Pryx("https://pryx.test","private-key",transport=httpx.MockTransport(handler)) as client:
            with self.assertRaises(PryxError) as error: client.make({"image":"alpine:3.22"})
            self.assertEqual(error.exception.code,"no_room")
            self.assertEqual(len(calls),1)
