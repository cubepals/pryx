// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** The celld proposal authenticates its transport, keeps identities stable and strips bridge credentials before relaying. */
import { expect, test } from 'bun:test'
import { Pryx } from '@cubepals/pryx'
import { StatusCodes } from 'http-status-codes'
import { containerBridge } from '../src/index.ts'

test('first use is keyed, private forwarding wakes and never sends the bridge credential', async () => {
  const calls: Request[] = []
  const client = new Pryx({
    url: 'http://control',
    key: 'api-secret',
    fetch: (async (input, init) => {
      const request = new Request(input, init)
      calls.push(request)
      return Response.json(
        request.url.endsWith('/endpoints')
          ? { endpoints: [{ name: 'http', host: '10.0.0.5', port: 40001 }] }
          : { id: 'sandbox' },
      )
    }) as typeof fetch,
  })
  const bridge = containerBridge(client, {
    token: 'bridge-secret',
    image: 'host-policy-image',
    port: 8080,
    idle: 60,
    send: (async (url, init) => {
      expect(String(url)).toBe('http://10.0.0.5:40001/path?q=ok')
      expect(new Headers(init?.headers).has('authorization')).toBe(false)
      return new Response('container')
    }) as typeof fetch,
  })
  expect((await bridge.request('/containers', { method: 'POST' })).status).toBe(StatusCodes.UNAUTHORIZED)
  const headers = { authorization: 'Bearer bridge-secret', 'content-type': 'application/json' }
  await bridge.request('/containers', {
    method: 'POST',
    headers,
    body: JSON.stringify({ cell: 'cell-id:0' }),
  })
  expect(calls[0]?.headers.get('idempotency-key')).toBe('celld:cell-id:0')
  const first = calls[0]
  if (!first) throw new Error('Missing create request')
  expect((await first.json()).idle).toBe(60)
  const response = await bridge.request('/containers/sandbox/fetch/path?q=ok', { headers })
  expect(await response.text()).toBe('container')
  expect(new URL(calls[2]?.url ?? '').pathname).toBe('/v1/sandboxes/sandbox/wake')
})
