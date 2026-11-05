// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** A real HTTP/TLS sandbox sleeps and wakes through the edge; route epoch replacement closes live sockets in CI. */
import { afterAll, beforeAll, describe, expect, test } from 'bun:test'
import { request } from 'node:https'
import { type AddressInfo, connect } from 'node:net'
import { Pryx } from '@cubepals/pryx'
import { startEdge } from '../../edge/src/agent.ts'
import { edgeConfig } from '../../edge/src/config.ts'
import { MINUTE_MS } from '../src/durations.ts'
import { deployment, ready } from './deployment.ts'
import { until } from './waits.ts'

describe.skipIf(!ready)('edge through a real pryxd', () => {
  let d: Awaited<ReturnType<typeof deployment>>, edge: Awaited<ReturnType<typeof startEdge>>, client: Pryx
  beforeAll(async () => {
    d = await deployment()
    client = new Pryx({ baseUrl: d.control.apiUrl, apiKey: d.credential.key })
    edge = await startEdge({
      ...edgeConfig({
        PRYX_CONTROL_URL: d.control.apiUrl,
        PRYX_EDGE_TOKEN: d.control.config.edgeToken,
        PRYX_EDGE_HOST: '127.0.0.1',
      }),
      httpPort: 0,
      tlsPort: 0,
      metricsPort: 0,
      holdMs: (15 * MINUTE_MS) / 60,
    })
    await edge.poll.firstHold
  }, 2 * MINUTE_MS)
  afterAll(async () => {
    await edge?.stop()
    await d?.close()
  })
  const port = (key: string) => (edge.listeners.get(key)?.address() as AddressInfo).port
  test(
    'HTTP requests replay after CLI sleep, and a higher route epoch closes the old connection',
    async () => {
      const made = await client.make({
        image: 'alpine:pryx-tls',
        entrypoint: [
          'sh',
          '-c',
          'echo hello > /data/index.html; dd if=/dev/zero of=/data/hold bs=1048576 count=8; exec httpd -f -p 8080 -h /data',
        ],
        resources: { memoryMb: 128 },
        ports: [{ name: 'http', port: 8080, audience: 'public' }],
      })
      try {
        await until('edge holds HTTP', async () => edge.table.held.get(`${made.id}:http`))
        const call = () =>
          fetch(`http://127.0.0.1:${port('http:80')}/`, { headers: { host: `${made.id}.pryx.test` } }).then(
            (r) => r.text(),
          )
        expect((await call()).trim()).toBe('hello')
        const command = Bun.spawn(['bun', 'cli/src/main.ts', 'sandbox', 'sleep', made.id], {
          env: d.env,
          stdout: 'ignore',
          stderr: 'pipe',
        })
        expect(await command.exited).toBe(0)
        await until(
          'edge holds asleep',
          async () => edge.table.held.get(`${made.id}:http`)?.state === 'asleep',
        )
        expect((await call()).trim()).toBe('hello')
        console.log('HTTP request woke the sandbox and completed')
        const socket = connect({ host: '127.0.0.1', port: port('http:80') })
        socket.on('error', () => {})
        // Leave a large response unread so backpressure keeps the relay live until the epoch changes.
        socket.write(`GET /hold HTTP/1.1\r\nHost: ${made.id}.pryx.test\r\n\r\n`)
        await until('spliced connection', async () =>
          [...edge.table.connections].find((c) => c.sandbox === made.id),
        )
        const route = edge.table.held.get(`${made.id}:http`)
        if (!route) throw new Error('Missing route')
        const closed = new Promise<void>((resolve) => socket.once('close', () => resolve()))
        edge.table.apply({
          routes: [...edge.table.held.values()].map((r) =>
            r.id === route.id ? { ...r, epoch: r.epoch + 1, destination: '127.0.0.1:1' } : r,
          ),
        })
        // Drain bytes already queued on the paused client so Node can observe the peer's FIN.
        socket.resume()
        await closed
        expect([...edge.table.connections].some((connection) => connection.sandbox === made.id)).toBe(false)
      } finally {
        await client.delete(made.id)
      }
    },
    2 * MINUTE_MS,
  )
  test(
    'HTTPS preserves ClientHello and completes after a sleeping sandbox wakes',
    async () => {
      const alias = `tls-${crypto.randomUUID().slice(0, 8)}`,
        hostname = `${alias}.pryx.test`,
        identity = await d.control.ca.identity(hostname, 'server', 1)
      const made = await client.make({
        image: 'alpine:pryx-tls',
        entrypoint: [
          'sh',
          '-c',
          'printf %s "$CERT" | base64 -d > /data/cert.pem; printf %s "$KEY" | base64 -d > /data/key.pem; chmod 600 /data/key.pem; exec openssl s_server -accept 8080 -cert /data/cert.pem -key /data/key.pem -www -quiet',
        ],
        secrets: {
          CERT: Buffer.from(identity.certPem).toString('base64'),
          KEY: Buffer.from(identity.keyPem).toString('base64'),
        },
        resources: { memoryMb: 128 },
        ports: [{ name: 'tls', port: 8080, audience: 'public', front: 'tls', alias }],
      })
      const call = () =>
        new Promise<string>((resolve, reject) => {
          const req = request(
            {
              host: '127.0.0.1',
              port: port('tls:443'),
              servername: hostname,
              ca: d.control.ca.pem,
              agent: false,
            },
            (r) => {
              let text = ''
              r.on('data', (d) => {
                text += d
              })
              r.on('end', () => resolve(text))
            },
          )
          req.on('error', reject)
          req.end()
        })
      try {
        await until('edge holds TLS', async () => edge.table.held.get(`${made.id}:tls`))
        expect(await call()).toContain('s_server')
        await client.action(made.id, 'sleep')
        await until(
          'edge holds TLS asleep',
          async () => edge.table.held.get(`${made.id}:tls`)?.state === 'asleep',
        )
        expect(await call()).toContain('s_server')
        console.log('HTTPS request woke the sandbox and completed with the test CA')
      } finally {
        await client.delete(made.id)
      }
    },
    2 * MINUTE_MS,
  )
})
