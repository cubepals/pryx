// SPDX-License-Identifier: FSL-1.1-ALv2
/** API and agent requests can share one keep-alive connection on the image's single HTTP
 * origin; project credentials are forwarded only to the configured control listener. */
import { expect, test } from 'bun:test'
import { Agent, request } from 'node:http'
import { gzipSync } from 'node:zlib'
import { agentContext } from './agent-context.ts'

test('one connection switches between control and agent requests', async () => {
  const c = agentContext(),
    authorization: boolean[] = []
  const control = Bun.serve({
    hostname: '127.0.0.1',
    port: 0,
    fetch: (request) => {
      authorization.push(request.headers.get('authorization') === 'Bearer project-key')
      return new Response('control', {
        headers: { connection: 'close, x-upstream-only', 'x-upstream-only': 'private-hop' },
      })
    },
  })
  c.proxy.useControl(control.url.toString())
  const agent = new Agent({ keepAlive: true, maxSockets: 1 }),
    connections = new Set<number>()
  const get = (path: string, token: string) =>
    new Promise<string>((resolve, reject) => {
      const req = request(
        `${c.url.split('/sandboxes/')[0]}${path}`,
        { agent, headers: { authorization: `Bearer ${token}` } },
        (response) => {
          expect(response.headers['x-upstream-only']).toBeUndefined()
          expect(response.headers.connection).not.toBe('close')
          connections.add(response.socket.localPort as number)
          let body = ''
          response.on('data', (bytes) => {
            body += bytes
          })
          response.on('end', () => resolve(body))
        },
      )
      req.on('error', reject)
      req.end()
    })
  try {
    expect(await get('/v1/sandboxes', 'project-key')).toBe('control')
    expect(JSON.parse(await get('/sandboxes/one/_pryx/health', c.token()))).toEqual({ status: 'ok' })
    expect(await get('/v1/sandboxes', 'project-key')).toBe('control')
    expect(connections.size).toBe(1)
    expect(authorization).toEqual([true, true])
  } finally {
    agent.destroy()
    c.close()
    control.stop(true)
  }
})

test('relay preserves encoded response bytes and representation headers', async () => {
  const c = agentContext(),
    encoded = gzipSync('encoded application content')
  const upstream = Bun.serve({
    hostname: '127.0.0.1',
    port: 0,
    fetch: () =>
      new Response(encoded, {
        headers: {
          'content-encoding': 'gzip',
          'content-length': String(encoded.length),
          connection: 'close, x-upstream-only',
          'x-upstream-only': 'private-hop',
        },
      }),
  })
  c.proxy.useControl(upstream.url.toString())
  try {
    const response = await fetch(`${c.url.split('/sandboxes/')[0]}/v1/encoded`, { decompress: false })
    expect(response.headers.get('content-encoding')).toBe('gzip')
    expect(response.headers.get('content-length')).toBe(String(encoded.length))
    expect(response.headers.get('x-upstream-only')).toBeNull()
    expect(new Uint8Array(await response.arrayBuffer())).toEqual(new Uint8Array(encoded))
  } finally {
    c.close()
    upstream.stop(true)
  }
})
