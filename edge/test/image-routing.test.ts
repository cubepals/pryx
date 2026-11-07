// SPDX-License-Identifier: FSL-1.1-ALv2
/** The image's catch-all API fallback must coexist with explicitly declared HTTP hosts. */
import { expect, test } from 'bun:test'
import type { AddressInfo } from 'node:net'
import { AppRequests } from '../../control/e2e/app-requests.ts'
import { HTTP_FRONT_PORT } from '../../control/src/edge-headers.ts'
import { edgeConfig } from '../src/config.ts'
import { front } from '../src/front.ts'
import { Hold } from '../src/hold.ts'
import { Gate } from '../src/limits.ts'
import { Metrics } from '../src/metrics.ts'
import { agentContext } from './agent-context.ts'

test('image fallback preserves declared HTTP hosts and control and agent aliases', async () => {
  const c = agentContext()
  const app = Bun.serve({
    hostname: '127.0.0.1',
    port: 0,
    fetch: (request) => new Response(`declared:${new URL(request.url).pathname}`),
  })
  const control = Bun.serve({ hostname: '127.0.0.1', port: 0, fetch: () => new Response('control') })
  const declared = {
    id: 'one:web',
    sandbox: 'one',
    hostname: 'one.sandboxes.test',
    port: 'web',
    front: 'http' as const,
    publicPort: HTTP_FRONT_PORT,
    destination: `127.0.0.1:${app.port}`,
    epoch: c.route.epoch,
  }
  const colliding = { ...declared, id: 'other:web', sandbox: 'other', hostname: 'control.test' }
  c.table.apply({ routes: [c.route, declared, colliding] })
  c.proxy.useControl(control.url.toString())
  const config = edgeConfig({ PRYX_EDGE_TOKEN: 'x'.repeat(32) })
  const listener = await front({
    host: '127.0.0.1',
    port: 0,
    kind: 'http',
    routePort: HTTP_FRONT_PORT,
    table: c.table,
    agents: c.proxy,
    gate: new Gate(config.limits),
    hold: new Hold(async () => {
      throw Error('unexpected wake')
    }),
    holdMs: config.holdMs,
    greetingMs: config.greetingMs,
    bufferBytes: config.bufferBytes,
    metrics: new Metrics(),
    session: () => {},
    control: { destination: c.proxy.destination() as string, paths: /^\//, hostname: 'control.test' },
  })
  const origin = `http://127.0.0.1:${(listener.address() as AddressInfo).port}`
  const apps = new AppRequests(origin)
  try {
    expect(await (await apps.get(`http://${declared.hostname}/hello`)).text()).toBe('declared:/hello')
    expect(await (await fetch(`${origin}/v1/sandboxes`)).text()).toBe('control')
    expect(await (await apps.get('http://control.test/v1/sandboxes')).text()).toBe('control')
    const agent = await fetch(`${origin}/sandboxes/one/_pryx/health`, {
      headers: { authorization: `Bearer ${c.token()}` },
    })
    expect(await agent.json()).toEqual({ status: 'ok' })
  } finally {
    apps.close()
    listener.closeClients()
    await new Promise<void>((resolve) => listener.close(() => resolve()))
    c.close()
    app.stop(true)
    control.stop(true)
  }
})
