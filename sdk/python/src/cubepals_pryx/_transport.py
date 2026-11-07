# SPDX-License-Identifier: FSL-1.1-ALv2
"""Private verified HTTP pools and stable errors; no endpoint methods or automatic retries."""
import json
import httpx

class PryxError(Exception):
    def __init__(self, status, envelope):
        detail = envelope.get('error', {}) if isinstance(envelope, dict) else {}
        if not isinstance(detail, dict): detail = {}
        self.status, self.code, self.details = status, detail.get('code', 'http_error'), detail.get('details')
        super().__init__(detail.get('message', f'Request refused (HTTP {status})'))

def _checked(response):
    if not response.is_success:
        try: envelope = response.json()
        except ValueError: envelope = {}
        raise PryxError(response.status_code, envelope)
    return response

class Transport:
    def __init__(self, url, key, *, transport=None, timeout=None):
        self._key = key
        self.http = httpx.Client(base_url=url.rstrip('/'), transport=transport, timeout=timeout, follow_redirects=False)
    def close(self): self.http.close()
    def request(self, path, method='GET', body=None, key=None):
        response = _checked(self.http.request(method, path, json=body, headers={'authorization':f'Bearer {self._key}', **({'Idempotency-Key':key} if key else {})}))
        return response.json() if response.content else None
    def transfer(self, path, method='GET', content=None):
        return _checked(self.http.request(method, path, content=content, headers={'authorization':f'Bearer {self._key}'}))
    def lines(self, path):
        with self.http.stream('GET', path, headers={'authorization':f'Bearer {self._key}'}) as response:
            if not response.is_success: response.read(); _checked(response)
            for line in response.iter_lines():
                if line.strip(): yield json.loads(line)

class AsyncTransport:
    def __init__(self, url, key, *, transport=None, timeout=None):
        self._key = key
        self.http = httpx.AsyncClient(base_url=url.rstrip('/'), transport=transport, timeout=timeout, follow_redirects=False)
    async def close(self): await self.http.aclose()
    async def request(self, path, method='GET', body=None, key=None):
        response = _checked(await self.http.request(method, path, json=body, headers={'authorization':f'Bearer {self._key}', **({'Idempotency-Key':key} if key else {})}))
        return response.json() if response.content else None
    async def transfer(self, path, method='GET', content=None):
        return _checked(await self.http.request(method, path, content=content, headers={'authorization':f'Bearer {self._key}'}))
    async def lines(self, path):
        async with self.http.stream('GET', path, headers={'authorization':f'Bearer {self._key}'}) as response:
            if not response.is_success: await response.aread(); _checked(response)
            async for line in response.aiter_lines():
                if line.strip(): yield json.loads(line)
