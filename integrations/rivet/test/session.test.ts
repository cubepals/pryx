// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Exercise persisted create recovery and lifecycle callbacks from the real RivetKit definition against SDK HTTP transport. */
import { expect, test } from 'bun:test'
import { Pryx } from '@cubepals/pryx'
import { pryxProvider } from '../src/agentos.ts'
import { type SessionMemory, sandboxActor } from '../src/index.ts'

test('a lost create reply keeps identity; lifecycle manages its recovered sandbox', async () => {
  const calls: { path: string; key: string | null; body: unknown }[] = []
  let lose = true
  const client = new Pryx({
    baseUrl: 'http://fixture',
    apiKey: 'server-secret',
    fetch: (async (input, init) => {
      const path = new URL(String(input)).pathname
      calls.push({
        path,
        key: new Headers(init?.headers).get('idempotency-key'),
        body: init?.body ? JSON.parse(String(init.body)) : null,
      })
      if (lose) {
        lose = false
        throw new Error('lost after create')
      }
      return Response.json({ id: 'kept', endpoints: [], state: 'running' })
    }) as typeof fetch,
  })
  const definition = sandboxActor(client),
    config = definition.config
  const actions = config.actions
  if (!actions) throw new Error('Actor actions missing')
  const state: SessionMemory = { sandbox: null, pending: null, generation: 0, notes: [] }
  let saves = 0,
    sleeps = 0
  const context = {
    id: 'actor-id',
    state,
    saveState: async () => {
      saves++
    },
    sleep: () => {
      sleeps++
    },
  } as unknown as Parameters<typeof actions.make>[0]
  await expect(actions.make(context, { image: 'alpine:3.22' })).rejects.toThrow('lost')
  expect(state.pending?.key).toBe('rivet:actor-id:0')
  expect(saves).toBe(1)
  await actions.make(context, { image: 'different' })
  expect(calls[0]?.body).toEqual(calls[1]?.body)
  expect(calls[0]?.key).toBe(calls[1]?.key)
  await config.onSleep?.(context as unknown as Parameters<NonNullable<typeof config.onSleep>>[0])
  await config.onWake?.(context as unknown as Parameters<NonNullable<typeof config.onWake>>[0])
  await actions.sleep(context)
  expect(sleeps).toBe(1)
  await config.onDestroy?.(context as unknown as Parameters<NonNullable<typeof config.onDestroy>>[0])
  expect(calls.slice(2).map((c) => c.path)).toEqual([
    '/v1/sandboxes/kept/sleep',
    '/v1/sandboxes/kept/wake',
    '/v1/sandboxes/kept/sleep',
    '/v1/sandboxes/kept',
  ])
  expect(JSON.stringify(state)).not.toContain('server-secret')
})
test('sandbox-agent provider preserves keys, pauses and handles private IPv6 endpoints', async () => {
  const requests: Request[] = []
  const client = new Pryx({
    baseUrl: 'http://fixture',
    apiKey: 'server-secret',
    fetch: (async (input, init) => {
      const request = new Request(input, init)
      requests.push(request)
      return Response.json(
        request.url.endsWith('/endpoints')
          ? { endpoints: [{ name: 'agent', host: 'fd00::1', port: 1234 }] }
          : { id: 'one' },
      )
    }) as typeof fetch,
  })
  const provider = pryxProvider(client, {
    key: 'session-key',
    image: 'image-from-host-policy',
    agentPort: 1234,
  })
  expect(await provider.create()).toBe('one')
  expect(requests[0]?.headers.get('idempotency-key')).toBe('session-key')
  const first = requests[0]
  if (!first) throw new Error('Missing create request')
  expect((await first.json()).ports[0].audience).toBe('private')
  expect(await provider.getUrl?.('one')).toBe('http://[fd00::1]:1234')
  await provider.pause?.('one')
  await provider.reconnect?.('one')
  await provider.destroy('one')
  expect(requests.slice(2).map((r) => new URL(r.url).pathname)).toEqual([
    '/v1/sandboxes/one/sleep',
    '/v1/sandboxes/one/wake',
    '/v1/sandboxes/one',
  ])
})
