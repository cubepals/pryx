// SPDX-License-Identifier: FSL-1.1-ALv2
/** Agent authentication precedes wake, port changes reach the public front, and websocket relays retain output. */
import { expect, test } from 'bun:test'
import { setTimeout as pause } from 'node:timers/promises'
import { StatusCodes } from 'http-status-codes'
import { SECOND_MS } from '../../control/src/durations.ts'
import { Pryx } from '../../sdk/typescript/src/index.ts'
import { sandboxToken } from '../src/sandbox-token.ts'
import { agentContext } from './agent-context.ts'

test('invalid, expired, foreign and route-only capabilities cannot wake an agent', async () => {
  const c = agentContext()
  c.table.apply({ routes: [{ ...c.route, state: 'asleep' }] })
  try {
    const now = Math.floor(Date.now() / SECOND_MS)
    for (const token of [
      '',
      c.token({ sandbox: 'other' }),
      c.token({ epoch: 2 }),
      c.token({ expiresAt: now }),
      c.token({ scope: 'routes' }),
    ]) {
      const response = await c.proxy.fetch(
        new Request(`${c.url}/_pryx/health`, { headers: { authorization: `Bearer ${token}` } }),
      )
      expect(response?.status).toBe(StatusCodes.UNAUTHORIZED)
    }
    expect(c.wakes).toBe(0)
    expect(c.calls).toHaveLength(0)
    const response = await c.proxy.fetch(
      new Request(`${c.url}/_pryx/health`, { headers: { authorization: `Bearer ${c.token()}` } }),
    )
    expect(response?.status).toBe(StatusCodes.OK)
    await response?.text()
    expect(c.wakes).toBe(1)
    expect(sandboxToken(c.token(), { ...c.route, epoch: 2 })).toBeUndefined()
    expect(c.table.find(c.route.hostname, 'http', 80)).toBeUndefined()
  } finally {
    c.close()
  }
})

test('an exposed spare is public and the SDK terminal keeps initial output and stdin', async () => {
  const c = agentContext()
  const client = new Pryx({
    url: 'http://control',
    key: 'project-secret',
    fetch: (async (input, init) => {
      if (String(input).endsWith('/token'))
        return Response.json({
          token: c.token(),
          expiresAt: Date.now() / SECOND_MS + 60,
          url: c.url,
          publicKey: c.route.agent?.manifest.publicKey,
          epoch: 1,
        })
      if (String(input).startsWith('http://control/'))
        return Response.json({ id: 'one', state: 'running', endpoints: [] })
      return fetch(input, init)
    }) as typeof fetch,
  })
  try {
    const agent = await client.sandbox('one')
    expect(await agent.expose(3000)).toBe('https://one-3000.sandboxes.test')
    const response = await c.proxy.fetch(new Request('http://one-3000.sandboxes.test/'))
    expect(await response?.text()).toBe('exposed-app')
    const alias = await c.proxy.fetch(new Request(`${c.url}/ports/3000/`))
    expect(await alias?.text()).toBe('exposed-app')
    const unavailable = await c.proxy.fetch(new Request('http://one-3001.sandboxes.test/'))
    expect(unavailable?.status).toBe(StatusCodes.NOT_FOUND)
    const terminal = await agent.terminal({ cols: 120, rows: 40 })
    const output = terminal.stream()
    expect((await output.next()).value?.data).toBe('prompt\n')
    terminal.stdin.write('hello\n')
    expect((await output.next()).value?.data).toBe('hello\n')
    expect(c.frames).toContainEqual({ type: 'resize', cols: 120, rows: 40 })
    terminal.close()
    await output.return(undefined)
  } finally {
    c.close()
  }
})

for (const state of ['missing', 'expired'] as const) {
  test(`signed port changes reach the public front while the route token is ${state}`, async () => {
    const c = agentContext()
    try {
      if (!c.route.agent) throw new Error('Missing fixture agent')
      if (state === 'expired') {
        const now = Math.floor(Date.now() / SECOND_MS)
        c.route.agent.routeToken = c.token({ scope: 'routes', issuedAt: now - 1, expiresAt: now })
      } else delete c.route.agent.routeToken
      c.table.apply({ routes: [c.route] })
      const port = 3000
      const app = 'http://one-3000.sandboxes.test/'
      const ports = `${c.url}/_pryx/ports`
      const headers = { authorization: `Bearer ${c.token()}`, 'content-type': 'application/json' }
      expect((await c.proxy.fetch(new Request(app)))?.status).toBe(StatusCodes.NOT_FOUND)
      const added = await fetch(ports, { method: 'POST', headers, body: JSON.stringify({ port }) })
      expect(added.status).toBe(StatusCodes.OK)
      await added.body?.cancel()
      const exposed = await c.proxy.fetch(new Request(app))
      expect(exposed?.status).toBe(StatusCodes.OK)
      expect(await exposed?.text()).toBe('exposed-app')

      // A public lookup of an unassigned port cannot reuse the mutation's sandbox capability.
      const unassigned = await c.proxy.fetch(new Request('http://one-3001.sandboxes.test/'))
      expect(unassigned?.status).toBe(StatusCodes.NOT_FOUND)
      expect(c.calls).toEqual(['POST /_pryx/ports', 'GET /_pryx/ports'])

      const removed = await fetch(`${ports}/${port}`, { method: 'DELETE', headers })
      expect(removed.status).toBe(StatusCodes.NO_CONTENT)
      expect((await c.proxy.fetch(new Request(app)))?.status).toBe(StatusCodes.NOT_FOUND)
      expect(c.calls).toEqual([
        'POST /_pryx/ports',
        'GET /_pryx/ports',
        'DELETE /_pryx/ports/3000',
        'GET /_pryx/ports',
      ])
    } finally {
      c.close()
    }
  })
}

test('a failed optional port refresh cannot hide the accepted mutation', async () => {
  const c = agentContext()
  try {
    if (!c.route.agent) throw new Error('Missing fixture agent')
    delete c.route.agent.routeToken
    c.table.apply({ routes: [c.route] })
    c.hooks.ports = () => new Response('unreadable port list')
    const port = 3000
    const added = await fetch(`${c.url}/_pryx/ports`, {
      method: 'POST',
      headers: { authorization: `Bearer ${c.token()}`, 'content-type': 'application/json' },
      body: JSON.stringify({ port }),
    })
    expect(added.status).toBe(StatusCodes.OK)
    expect(await added.json()).toMatchObject({ port })
    expect((await c.proxy.fetch(new Request('http://one-3000.sandboxes.test/')))?.status).toBe(
      StatusCodes.NOT_FOUND,
    )
    expect(c.calls).toEqual(['POST /_pryx/ports', 'GET /_pryx/ports'])
  } finally {
    c.close()
  }
})

test('a port mutation replaces an earlier pending assignment refresh', async () => {
  const c = agentContext()
  const started = Promise.withResolvers<void>()
  const oldList = Promise.withResolvers<Response>()
  const timeout = new AbortController()
  const requests: Promise<Response>[] = []
  const port = 3000
  try {
    if (!c.route.agent) throw new Error('Missing fixture agent')
    delete c.route.agent.routeToken
    c.table.apply({ routes: [c.route] })
    c.hooks.ports = () => {
      if (c.calls.filter((call) => call === 'GET /_pryx/ports').length > 1) return
      started.resolve()
      return oldList.promise
    }
    const ports = `${c.url}/_pryx/ports`
    const headers = { authorization: `Bearer ${c.token()}`, 'content-type': 'application/json' }
    const added = fetch(ports, { method: 'POST', headers, body: JSON.stringify({ port }) })
    requests.push(added)
    expect(
      await Promise.race([
        started.promise.then(() => true),
        pause(SECOND_MS, false, { signal: timeout.signal }),
      ]),
    ).toBe(true)
    const removed = fetch(`${ports}/${port}`, { method: 'DELETE', headers })
    requests.push(removed)
    const confirmation = await Promise.race([removed, pause(SECOND_MS, null, { signal: timeout.signal })])
    expect(confirmation?.status).toBe(StatusCodes.NO_CONTENT)
    const app = 'http://one-3000.sandboxes.test/'
    oldList.resolve(Response.json({ ports: [{ port, slot: 1, url: app }] }))
    expect((await added).status).toBe(StatusCodes.OK)
    expect((await c.proxy.fetch(new Request(app)))?.status).toBe(StatusCodes.NOT_FOUND)
  } finally {
    timeout.abort()
    oldList.resolve(Response.json({ ports: [] }))
    for (const pending of requests)
      await pending.then(
        (response) => response.body?.cancel(),
        () => {},
      )
    c.close()
  }
})
