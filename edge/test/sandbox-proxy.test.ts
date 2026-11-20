// SPDX-License-Identifier: FSL-1.1-ALv2
/** Agent authentication precedes wake, port changes reach the public front, and websocket relays retain output. */
import { expect, test } from 'bun:test'
import { setTimeout as pause } from 'node:timers/promises'
import { StatusCodes } from 'http-status-codes'
import { SECOND_MS } from '../../control/src/durations.ts'
import node from '../../openapi/node-writes.openapi.json'
import { TOOL_DEFAULTS } from '../../sdk/typescript/src/generated/tools.ts'
import { Pryx } from '../../sdk/typescript/src/index.ts'
import { requestToken, sandboxToken } from '../src/sandbox-token.ts'
import { agentContext } from './agent-context.ts'

function expiredRouteToken(c: ReturnType<typeof agentContext>) {
  const now = Math.floor(Date.now() / SECOND_MS)
  return c.token({ scope: 'routes', issuedAt: now - 1, expiresAt: now })
}

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

test('a bound file capability reaches its file after wake without an optional route capability', async () => {
  const c = agentContext()
  try {
    if (!c.route.agent) throw new Error('Missing fixture agent')
    delete c.route.agent.routeToken
    const path = 'message'
    const url = `${c.url}/_pryx/files/content?path=${path}`
    const put = await fetch(url, {
      method: 'PUT',
      headers: { authorization: `Bearer ${c.token()}` },
      body: 'file contents',
    })
    expect(put.status).toBe(StatusCodes.NO_CONTENT)
    c.table.apply({ routes: [{ ...c.route, state: 'asleep' }] })
    const issuedAt = Math.floor(Date.now() / SECOND_MS)
    const credential = c.token({
      scope: 'file',
      issuedAt,
      expiresAt: issuedAt + node['x-pryx-agent'].fileTokenLifetimeSeconds,
      file: { method: 'GET', path },
    })
    const files = await fetch(`${url}&token=${credential}`, { signal: AbortSignal.timeout(SECOND_MS) })
    expect(files.status).toBe(StatusCodes.OK)
    expect(await files.text()).toBe('file contents')
    expect(c.wakes).toBe(1)
    expect(c.calls).toEqual(['PUT /_pryx/files/content', 'GET /_pryx/files/content'])
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
      if (state === 'expired') c.route.agent.routeToken = expiredRouteToken(c)
      else delete c.route.agent.routeToken
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
      const stillExposed = await c.proxy.fetch(new Request(app))
      expect(stillExposed?.status).toBe(StatusCodes.OK)
      expect(await stillExposed?.text()).toBe('exposed-app')
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

for (const tool of ['browser', 'desktop'] as const) {
  test(`${tool} access is ready before the edge receives a route token or port update`, async () => {
    const c = agentContext()
    try {
      if (!c.route.agent) throw new Error('Missing fixture agent')
      delete c.route.agent.routeToken
      c.table.apply({ routes: [c.route] })
      c.hooks.app = (request) => {
        expect(request.headers.has('authorization')).toBe(false)
        return new URL(request.url).searchParams.get('token') === 'fixture-access'
          ? new Response('tool available')
          : new Response('tool access required', { status: StatusCodes.FORBIDDEN })
      }
      const endpoint = `${c.url}/_pryx/${tool}`
      const options = { method: 'POST', headers: { authorization: `Bearer ${c.token()}` } }
      await (await fetch(`${endpoint}/screenshot`, options)).body?.cancel()
      expect(c.calls).toEqual([`POST /_pryx/${tool}/screenshot`])

      if (tool === 'desktop') {
        c.route.agent.routeToken = expiredRouteToken(c)
        c.table.apply({ routes: [{ ...c.route, state: 'asleep' }] })
        c.hooks.wake = () => {}
      }
      const opened = await fetch(endpoint, { ...options, signal: AbortSignal.timeout(SECOND_MS) })
      expect(opened.status).toBe(StatusCodes.OK)
      const url = new URL(((await opened.json()) as { url: string }).url)
      if (tool === 'browser') {
        const withoutAccess = new URL(url)
        withoutAccess.search = ''
        expect((await c.proxy.fetch(new Request(withoutAccess)))?.status).toBe(StatusCodes.FORBIDDEN)
      } else {
        expect(c.wakes).toBe(1)
        expect(c.table.held.get(c.route.id)?.state).toBe('asleep')
      }
      const port = tool === 'browser' ? TOOL_DEFAULTS.browserPort : TOOL_DEFAULTS.desktopPort
      const publicPage = await c.proxy.fetch(
        new Request(`${c.url}/ports/${port}${url.pathname}${url.search}`),
      )
      expect(publicPage?.status).toBe(StatusCodes.OK)
      expect(await publicPage?.text()).toBe('tool available')
      expect(c.calls.filter((call) => call === 'GET /_pryx/ports')).toHaveLength(1)
    } finally {
      c.close()
    }
  })
}

for (const state of ['missing', 'expired'] as const) {
  test(`MCP port tools refresh immediately while the route token is ${state}`, async () => {
    const c = agentContext()
    try {
      if (!c.route.agent) throw new Error('Missing fixture agent')
      const mcp = (method: string, params?: unknown) =>
        fetch(`${c.url}/_pryx/mcp`, {
          method: 'POST',
          headers: { authorization: `Bearer ${c.token()}`, 'content-type': 'application/json' },
          body: JSON.stringify({ jsonrpc: '2.0', id: 1, method, params }),
        })
      const healthy = await mcp('tools/list')
      expect(healthy.status).toBe(StatusCodes.OK)
      await healthy.body?.cancel()
      expect(c.calls).toEqual(['POST /_pryx/mcp'])

      if (state === 'expired') c.route.agent.routeToken = expiredRouteToken(c)
      else delete c.route.agent.routeToken
      c.table.apply({ routes: [c.route] })
      const port = 3000
      const app = `${c.url}/ports/${port}/`
      const added = await mcp('tools/call', { name: 'expose', arguments: { port } })
      expect(added.status).toBe(StatusCodes.OK)
      await added.body?.cancel()
      expect(await (await c.proxy.fetch(new Request(app)))?.text()).toBe('exposed-app')
      const removed = await mcp('tools/call', { name: 'unexpose', arguments: { port } })
      expect(removed.status).toBe(StatusCodes.OK)
      await removed.body?.cancel()
      expect((await c.proxy.fetch(new Request(app)))?.status).toBe(StatusCodes.NOT_FOUND)
      expect(c.calls).toEqual([
        'POST /_pryx/mcp',
        'POST /_pryx/mcp',
        'GET /_pryx/ports',
        'POST /_pryx/mcp',
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

for (const change of ['port revision', 'agent destination'] as const) {
  test(`a new ${change} does not wait for an older assignment refresh`, async () => {
    const c = agentContext()
    const started = Promise.withResolvers<void>()
    const oldList = Promise.withResolvers<Response>()
    const timeout = new AbortController()
    const requests: Promise<Response | undefined>[] = []
    const port = 3000
    const app = `${c.url}/ports/${port}/`
    const assignment = { port, slot: 1, url: app }
    const revision = change === 'port revision' ? 1 : 0
    const assignments = () => Response.json({ revision, ports: [assignment] })
    let reads = 0
    const replacement =
      change === 'agent destination'
        ? Bun.serve({
            hostname: '127.0.0.1',
            port: 0,
            fetch(request) {
              expect(request.method).toBe('GET')
              expect(new URL(request.url).pathname).toBe('/_pryx/ports')
              expect(sandboxToken(requestToken(request), c.route)?.scope).toBe('routes')
              reads++
              return assignments()
            },
          })
        : undefined
    try {
      if (!c.route.agent) throw new Error('Missing fixture agent')
      c.hooks.ports = () => {
        if (++reads > 1 && !replacement) return assignments()
        started.resolve()
        return oldList.promise
      }
      const older = c.proxy.fetch(new Request(`${c.url}/ports/3001/`))
      requests.push(older)
      expect(
        await Promise.race([
          started.promise.then(() => true),
          pause(SECOND_MS, false, { signal: timeout.signal }),
        ]),
      ).toBe(true)
      const currentRoute = replacement
        ? { ...c.route, destination: `127.0.0.1:${replacement.port}` }
        : {
            ...c.route,
            agent: {
              ...c.route.agent,
              manifest: { ...c.route.agent.manifest, portRevision: revision, exposed: [assignment] },
            },
          }
      c.table.apply({ routes: [currentRoute] })
      const current = c.proxy.fetch(new Request(app))
      requests.push(current)
      const response = await Promise.race([current, pause(SECOND_MS, null, { signal: timeout.signal })])
      expect(response?.status).toBe(StatusCodes.OK)
      expect(await response?.text()).toBe('exposed-app')

      oldList.resolve(Response.json({ revision: 0, ports: [] }))
      expect((await older)?.status).toBe(StatusCodes.NOT_FOUND)
      expect(await (await c.proxy.fetch(new Request(app)))?.text()).toBe('exposed-app')
      expect(reads).toBe(2)
    } finally {
      timeout.abort()
      oldList.resolve(Response.json({ revision: 0, ports: [] }))
      for (const pending of requests)
        await pending.then(
          (response) => (response?.bodyUsed ? undefined : response?.body?.cancel()),
          () => {},
        )
      replacement?.stop(true)
      c.close()
    }
  })
}

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
