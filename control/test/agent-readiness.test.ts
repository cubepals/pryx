// SPDX-License-Identifier: FSL-1.1-ALv2
/** Real HTTP admission during a cold agent route. Protocol peers model process execution;
 * this proves readiness traffic preserves the unchanged edge limits for the next client. */
import { expect, test } from 'bun:test'
import { request } from 'node:http'
import type { AddressInfo } from 'node:net'
import { StatusCodes } from 'http-status-codes'
import { edgeConfig } from '../../edge/src/config.ts'
import { front } from '../../edge/src/front.ts'
import { Gate } from '../../edge/src/limits.ts'
import { Metrics } from '../../edge/src/metrics.ts'
import { SECOND_MS } from '../src/durations.ts'
import { HTTP_FRONT_PORT } from '../src/edge-headers.ts'
import { publicContext } from './public-context.ts'

test('cold route readiness leaves admission for a separate command client', async () => {
  const c = await publicContext()
  const made = await (await c.request('/sandboxes', 'POST', { image: 'alpine:3.22' })).json()
  const peer = c.peers.get(made.id)
  if (!peer) throw Error('Missing agent protocol peer')
  peer.table.apply({ routes: [] })
  const config = edgeConfig({ PRYX_EDGE_TOKEN: 'fixture'.repeat(8) })
  const gate = new Gate(config.limits)
  const listener = await front({
    host: '127.0.0.1',
    port: 0,
    kind: 'http',
    routePort: HTTP_FRONT_PORT,
    table: peer.table,
    agents: peer.proxy,
    hold: peer.proxy.hold,
    gate,
    holdMs: config.holdMs,
    greetingMs: config.greetingMs,
    bufferBytes: config.bufferBytes,
    metrics: new Metrics(),
    session: () => {},
  })
  c.agents.localOrigin = `http://127.0.0.1:${(listener.address() as AddressInfo).port}`
  const publish = setTimeout(() => peer.table.apply({ routes: [peer.route] }), SECOND_MS)
  try {
    const credential = await c.agents.token(made.id, 'default', 'http://127.0.0.1/v1')
    // Node's agent:false forces a new connection, as the separate CLI process uses in quickstart.
    const status = await new Promise<number | undefined>((resolve, reject) => {
      const call = request(
        `${credential.url}/_pryx/processes`,
        {
          method: 'POST',
          agent: false,
          headers: { authorization: `Bearer ${credential.token}`, 'content-type': 'application/json' },
        },
        (response) => {
          response.resume()
          response.on('end', () => resolve(response.statusCode))
        },
      )
      call.once('error', (error) => {
        console.error(
          'Readiness admission refusals',
          [...gate.sweep()].map(([key, count]) => ({
            reason: key.split(' ')[0],
            count,
          })),
        )
        reject(error)
      })
      call.end(JSON.stringify({ command: ['echo', 'hello'] }))
    })
    expect(status).toBe(StatusCodes.CREATED)
    expect(peer.calls.filter((call) => call === 'POST /_pryx/processes')).toHaveLength(1)
    expect([...gate.sweep()].filter(([key]) => key.startsWith('address-rate '))).toEqual([])
  } finally {
    clearTimeout(publish)
    listener.closeClients()
    await new Promise<void>((resolve) => listener.close(() => resolve()))
    await c.close()
  }
})
