// SPDX-License-Identifier: FSL-1.1-ALv2
/** Exercise logical-origin pools with real client and server sockets. */
import { expect, test } from 'bun:test'
import { AppRequests } from '../e2e/app-requests.ts'

test('application fixtures reuse each origin without sharing its routed connection', async () => {
  const server = Bun.serve({
    hostname: '127.0.0.1',
    port: 0,
    fetch: (request, server) =>
      Response.json({
        host: request.headers.get('host'),
        path: new URL(request.url).pathname + new URL(request.url).search,
        port: server.requestIP(request)?.port,
      }),
  })
  const apps = new AppRequests(server.url.toString())
  try {
    const first = await (await apps.get('http://one-3000.pryx.test/a?q=1')).json()
    const other = await (await apps.get('http://two-3000.pryx.test/b')).json()
    const last = await (await apps.get('http://one-3000.pryx.test/c')).json()
    expect(first.host).toBe('one-3000.pryx.test')
    expect(first.path).toBe('/a?q=1')
    expect(other.host).toBe('two-3000.pryx.test')
    expect(last.port).toBe(first.port)
    expect(other.port).not.toBe(first.port)
  } finally {
    apps.close()
    server.stop(true)
  }
})
