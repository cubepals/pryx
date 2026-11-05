# SPDX-License-Identifier: FSL-1.1-ALv2
"""Public client over generated models and HTTP methods, with NDJSON streaming and stable errors."""
from __future__ import annotations
from http import HTTPStatus
from typing import Any, Iterator
import json
import httpx
from .generated import AuthenticatedClient
from .generated.api.default import (
    post_v1_sandboxes, get_v1_sandboxes_id, delete_v1_sandboxes_id,
    post_v1_sandboxes_id_snapshots, post_v1_snapshots_snapshot_sandboxes,
)
from .generated.models import MakeRequest
from .generated.types import UNSET

class PryxError(Exception):
    def __init__(self, status: int, envelope: dict[str, Any]):
        detail=envelope.get("error", {}) if isinstance(envelope,dict) else {}
        if not isinstance(detail,dict): detail={}
        self.status=status
        self.code=detail.get("code", "http_error")
        self.details=detail.get("details")
        super().__init__(detail.get("message", "Request refused"))

def _checked(response: httpx.Response):
    if response.is_error:
        try: envelope=response.json()
        except ValueError: envelope={}
        raise PryxError(response.status_code, envelope)
    return response

def _must(result):
    if not HTTPStatus.OK <= result.status_code < HTTPStatus.MULTIPLE_CHOICES:
        try: envelope=json.loads(result.content)
        except ValueError: envelope={}
        raise PryxError(result.status_code, envelope)
    return result.parsed

class Pryx:
    def __init__(self, base_url: str, api_key: str, *, timeout: httpx.Timeout | None=None, transport=None):
        self._client=AuthenticatedClient(base_url=base_url.rstrip("/"),token=api_key,timeout=timeout,httpx_args={"transport":transport} if transport else {})
    def close(self): self._client.get_httpx_client().close()
    def __enter__(self): return self
    def __exit__(self, *_): self.close()
    def make(self, body: dict[str, Any] | MakeRequest, key: str | None=None):
        return _must(post_v1_sandboxes.sync_detailed(client=self._client,body=MakeRequest.from_dict(body) if isinstance(body,dict) else body,idempotency_key=key if key is not None else UNSET))
    def get(self, sandbox: str): return _must(get_v1_sandboxes_id.sync_detailed(sandbox,client=self._client))
    def delete(self, sandbox: str): return _must(delete_v1_sandboxes_id.sync_detailed(sandbox,client=self._client))
    def snapshot(self, sandbox: str): return _must(post_v1_sandboxes_id_snapshots.sync_detailed(sandbox,client=self._client))
    def fork(self, snapshot: str, body: dict[str, Any] | None=None, key: str | None=None):
        return _must(post_v1_snapshots_snapshot_sandboxes.sync_detailed(snapshot,client=self._client,body=MakeRequest.from_dict(body or {}),idempotency_key=key if key is not None else UNSET))
    def request(self, path: str, method="GET", body=None):
        return _checked(self._client.get_httpx_client().request(method,path,json=body)).json()
    def list(self): return self.request("/v1/sandboxes")
    def action(self, sandbox: str, verb: str):
        if verb not in ("start","stop","sleep","wake","rest"): raise ValueError("Invalid sandbox action")
        return self.request(f"/v1/sandboxes/{sandbox}/{verb}","POST")
    def exec(self, sandbox: str, command: list[str], *, timeout: int | None=None, env: dict[str,str] | None=None, key: str | None=None) -> Iterator[dict[str,Any]]:
        body={"command":command,**({"timeout":timeout} if timeout is not None else {}),**({"env":env} if env is not None else {})}
        yield from self._lines(f"/v1/sandboxes/{sandbox}/exec","POST",body,{"Idempotency-Key":key} if key else {})
    def logs(self, sandbox: str, *, tail: int | None=None, since: str | None=None, follow=False):
        params={k:str(v).lower() if isinstance(v,bool) else str(v) for k,v in {"tail":tail,"since":since,"follow":follow}.items() if v is not None}
        yield from self._lines(f"/v1/sandboxes/{sandbox}/logs?{httpx.QueryParams(params)}")
    def _lines(self, path: str, method="GET", body=None, headers=None):
        with self._client.get_httpx_client().stream(method,path,json=body,headers=headers) as response:
            if response.is_error: response.read();_checked(response)
            for line in response.iter_lines():
                if line.strip(): yield json.loads(line)
    def snapshots(self, sandbox): return self.request(f"/v1/sandboxes/{sandbox}/snapshots")
    def delete_snapshot(self, sandbox, snapshot): return self.request(f"/v1/sandboxes/{sandbox}/snapshots/{snapshot}","DELETE")
    def endpoints(self, sandbox): return self.request(f"/v1/sandboxes/{sandbox}/endpoints")
    def events(self, sandbox, since=None): return self.request(f"/v1/sandboxes/{sandbox}/events"+(f"?{httpx.QueryParams({'since':since})}" if since else ""))
    def put_file(self, sandbox, path, data_base64): return self.request(f"/v1/sandboxes/{sandbox}/files/{path}","PUT",{"dataBase64":data_base64})
    def get_file(self, sandbox, path): return self.request(f"/v1/sandboxes/{sandbox}/files/{path}")
    def template(self, name, spec): return self.request("/v1/templates","POST",{"name":name,"spec":spec})
    def templates(self): return self.request("/v1/templates")
