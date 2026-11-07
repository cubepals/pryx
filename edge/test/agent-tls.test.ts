// SPDX-License-Identifier: FSL-1.1-ALv2
/** A verified TLS client reaches the agent through SNI; TLS handshakes and refused tokens
 * cannot wake compute. Ordinary sandbox TLS routes retain their existing pass-through path. */
import { expect, test } from 'bun:test'
import { request } from 'node:https'
import type { AddressInfo } from 'node:net'
import { StatusCodes } from 'http-status-codes'
import { FleetCa } from '../../control/src/ca.ts'
import { SECOND_MS } from '../../control/src/durations.ts'
import { TLS_FRONT_PORT } from '../../control/src/edge-headers.ts'
import { front } from '../src/front.ts'
import { Gate } from '../src/limits.ts'
import { Metrics } from '../src/metrics.ts'
import { agentContext } from './agent-context.ts'

test(
  'verified agent TLS authenticates before waking',
  async () => {
    const c = agentContext()
    const pems = await FleetCa.generate('agent-tls-test')
    const ca = await FleetCa.fromPem(pems.certPem, pems.keyPem)
    const identity = await ca.identity(c.route.hostname, 'server', 1)
    c.proxy.enableTls({ cert: identity.certPem, key: identity.keyPem })
    c.table.apply({ routes: [{ ...c.route, state: 'asleep' }] })
    const listener = await front({
      host: '127.0.0.1',
      port: 0,
      kind: 'tls',
      routePort: TLS_FRONT_PORT,
      agents: c.proxy,
      table: c.table,
      hold: c.proxy.hold,
      gate: new Gate({
        address: { connections: { perSecond: 5, burst: 30 }, greeting: 10, open: 64 },
        server: { connections: { perSecond: 20, burst: 100 }, open: 300 },
      }),
      holdMs: 5 * SECOND_MS,
      greetingMs: SECOND_MS,
      bufferBytes: 64 * 1024,
      metrics: new Metrics(),
      session: () => {},
    })
    const get = (token?: string) =>
      new Promise<number>((resolve, reject) => {
        const req = request(
          {
            host: '127.0.0.1',
            port: (listener.address() as AddressInfo).port,
            servername: c.route.hostname,
            ca: ca.pem,
            agent: false,
            path: '/_pryx/health',
            headers: { host: c.route.hostname, ...(token ? { authorization: `Bearer ${token}` } : {}) },
          },
          (response) => {
            response.resume()
            response.on('end', () => resolve(response.statusCode as number))
          },
        )
        req.on('error', reject)
        req.end()
      })
    try {
      expect(await get()).toBe(StatusCodes.UNAUTHORIZED)
      expect(c.wakes).toBe(0)
      expect(await get(c.token())).toBe(StatusCodes.OK)
      expect(c.wakes).toBe(1)
    } finally {
      listener.closeClients()
      await new Promise<void>((resolve) => listener.close(() => resolve()))
      c.close()
    }
  },
  10 * SECOND_MS,
)
