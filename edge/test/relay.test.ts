// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Generic fronts replay exact bytes, share wakes and sever stale epochs through real local TCP sockets. */

import { expect, test } from 'bun:test'
import { request, createServer as tlsServer } from 'node:https'
import { type AddressInfo, connect, type Socket } from 'node:net'
import { FleetCa } from '../../control/src/ca.ts'
import { SECOND_MS } from '../../control/src/durations.ts'

import { HTTP_FRONT_PORT, TLS_FRONT_PORT } from '../../control/src/edge-headers.ts'
import { front } from '../src/front.ts'
import { httpHost, tlsSni } from '../src/greeters.ts'
import { Hold } from '../src/hold.ts'
import { Gate } from '../src/limits.ts'
import { Metrics } from '../src/metrics.ts'
import { RouteTable } from '../src/routes.ts'
import type { EdgeRoute, WakeResult } from '../src/wire.ts'

const route: EdgeRoute = {
  id: 's:http',
  sandbox: 's',
  hostname: 's.test',
  port: 'http',
  front: 'http',
  publicPort: HTTP_FRONT_PORT,
  destination: '127.0.0.1:8080',
  epoch: 1,
}
const gate = () =>
  new Gate({
    address: { connections: { perSecond: 5, burst: 30 }, greeting: 10, open: 64 },
    server: { connections: { perSecond: 20, burst: 100 }, open: 300 },
  })
const port = (server: { address: () => unknown }) => (server.address() as AddressInfo).port
const close = (server: { close: (callback: () => void) => unknown }) =>
  new Promise<void>((resolve) => server.close(resolve))
test('HTTP greeting waits for complete headers and rejects ambiguity', () => {
  expect(httpHost(Buffer.from('GET / HTTP/1.1\r\nHost: S.Test:80\r\n'))).toBeNull()
  expect(httpHost(Buffer.from('GET / HTTP/1.1\r\nHost: S.Test:80\r\n\r\nbody'))).toBe('s.test')
  expect(() => httpHost(Buffer.from('GET / HTTP/1.1\r\nHost: s.test\r\nHost: other\r\n\r\n'))).toThrow()
  expect(tlsSni(Buffer.alloc(4))).toBeNull()
})

test('a large upload pauses at its greeting while a wake is outstanding', async () => {
  const payload = new Uint8Array(4 * 1024 * 1024).fill(42)
  const upstream = Bun.serve({
    hostname: '127.0.0.1',
    port: 0,
    async fetch(request) {
      const body = new Uint8Array(await request.arrayBuffer())
      expect(body).toEqual(payload)
      return new Response('received')
    },
  })
  const table = new RouteTable()
  const asleep = { ...route, state: 'asleep' as const, destination: `127.0.0.1:${upstream.port}` }
  table.apply({ routes: [asleep] })
  const hold = new Hold(async () => {
    await Bun.sleep(SECOND_MS / 10)
    return { outcome: 'ready', destination: asleep.destination, epoch: 1 }
  })
  const listener = await front({
    host: '127.0.0.1',
    port: 0,
    kind: 'http',
    routePort: HTTP_FRONT_PORT,
    table,
    hold,
    gate: gate(),
    holdMs: SECOND_MS,
    greetingMs: SECOND_MS,
    bufferBytes: 64 * 1024,
    metrics: new Metrics(),
    session: () => {},
  })
  try {
    const response = await fetch(`http://127.0.0.1:${port(listener)}/`, {
      method: 'PUT',
      headers: { host: route.hostname },
      body: payload,
    })
    expect(await response.text()).toBe('received')
  } finally {
    listener.closeClients()
    await close(listener)
    upstream.stop(true)
  }
})
test('a later epoch closes old connections and a delayed route cannot restore them', () => {
  let closed = 0
  const table = new RouteTable()
  table.apply({ routes: [route] })
  table.track(route, route.destination, 1, () => closed++)
  table.apply({ routes: [{ ...route, epoch: 2, destination: '127.0.0.1:8081' }] })
  expect(closed).toBe(1)
  table.apply({ routes: [route] })
  expect(table.held.get(route.id)?.epoch).toBe(2)
  table.apply({ routes: [] })
  table.apply({ routes: [route] })
  expect(table.find('s.test', 'http', HTTP_FRONT_PORT)).toBeUndefined()
})
test('waiting connections share a wake even if the first leaves', async () => {
  let asks = 0
  const answer = Promise.withResolvers<WakeResult>()
  const hold = new Hold(async () => {
      asks++
      return answer.promise
    }),
    first = new AbortController(),
    second = new AbortController()
  const a = hold.ready(route, first.signal).catch(() => null),
    b = hold.ready(route, second.signal)
  first.abort()
  answer.resolve({ outcome: 'ready', destination: route.destination, epoch: 1 })
  expect(await a).toBeNull()
  expect((await b).destination).toBe(route.destination)
  expect(asks).toBe(1)
  await expect(
    new Hold(async () => ({ outcome: 'denied', reason: 'unknown' })).ready(
      route,
      new AbortController().signal,
    ),
  ).rejects.toThrow()
})
test(
  'HTTP hold replays the body and TLS forwards the original ClientHello',
  async () => {
    const pems = await FleetCa.generate('edge-test'),
      ca = await FleetCa.fromPem(pems.certPem, pems.keyPem),
      identity = await ca.identity('s.test', 'server', 1)
    const tls = tlsServer({ cert: identity.certPem, key: identity.keyPem }, (_req, res) => {
      res.end('tls-ok')
    })
    await new Promise<void>((resolve) => tls.listen(0, '127.0.0.1', resolve))
    const http = Bun.serve({
      hostname: '127.0.0.1',
      port: 0,
      fetch: async (r) => new Response(`http:${await r.text()}`),
    })
    const table = new RouteTable(),
      metrics = new Metrics()
    const tlsRoute = {
      ...route,
      id: 's:tls',
      front: 'tls' as const,
      publicPort: TLS_FRONT_PORT,
      destination: `127.0.0.1:${port(tls)}`,
    }
    const httpRoute = { ...route, state: 'asleep' as const, destination: `127.0.0.1:${http.port}` }
    table.apply({ routes: [httpRoute, tlsRoute] })
    const hold = new Hold(async (id) => ({
      outcome: 'ready',
      destination: id === route.id ? httpRoute.destination : tlsRoute.destination,
      epoch: 1,
    }))
    const options = {
      host: '127.0.0.1',
      port: 0,
      table,
      gate: gate(),
      hold,
      holdMs: SECOND_MS,
      greetingMs: SECOND_MS,
      bufferBytes: 65536,
      metrics,
      session: () => {},
    }
    const h = await front({ ...options, kind: 'http', routePort: HTTP_FRONT_PORT }),
      t = await front({ ...options, kind: 'tls', routePort: TLS_FRONT_PORT })
    try {
      const response = await fetch(`http://127.0.0.1:${port(h)}/`, {
        method: 'POST',
        headers: { host: 's.test' },
        body: 'replayed',
      })
      expect(await response.text()).toBe('http:replayed')
      const body = await new Promise<string>((resolve, reject) => {
        const req = request(
          { host: '127.0.0.1', port: port(t), servername: 's.test', ca: ca.pem, agent: false },
          (res) => {
            let text = ''
            res.on('data', (d) => {
              text += d
            })
            res.on('end', () => resolve(text))
          },
        )
        req.on('error', reject)
        req.end()
      })
      expect(body).toBe('tls-ok')
    } finally {
      h.closeClients()
      t.closeClients()
      await close(h)
      await close(t)
      http.stop(true)
      await close(tls)
    }
  },
  10 * SECOND_MS,
)
test('a spliced raw connection survives the former hold deadline', async () => {
  const peers = new Set<Socket>(),
    backend = (await import('node:net')).createServer((s) => {
      peers.add(s)
      s.on('data', (d) => s.write(d))
      s.once('close', () => peers.delete(s))
    })
  await new Promise<void>((resolve) => backend.listen(0, '127.0.0.1', resolve))
  const table = new RouteTable(),
    r = { ...route, front: 'port' as const, publicPort: 20000, destination: `127.0.0.1:${port(backend)}` }
  table.apply({ routes: [r] })
  const listener = await front({
    host: '127.0.0.1',
    port: 0,
    routePort: 20000,
    kind: 'port',
    table,
    gate: gate(),
    hold: new Hold(async () => ({ outcome: 'ready', destination: r.destination, epoch: 1 })),
    holdMs: SECOND_MS / 10,
    greetingMs: SECOND_MS,
    bufferBytes: 65536,
    metrics: new Metrics(),
    session: () => {},
  })
  const client = connect({ host: '127.0.0.1', port: port(listener) })
  client.on('error', () => {})
  try {
    await Bun.sleep(SECOND_MS / 4)
    const response = new Promise<string>((resolve) => client.once('data', (d) => resolve(d.toString())))
    client.write('alive')
    expect(await response).toBe('alive')
  } finally {
    client.destroy()
    listener.closeClients()
    for (const s of peers) s.destroy()
    await close(listener)
    await close(backend)
  }
})
