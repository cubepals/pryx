// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Proposal transport for celld Workers: keyed make, explicit sleep/wake, and an HTTP port relay through the public SDK. */
import type { MakeRequest, Pryx } from '@cubepals/pryx'
import { Hono } from 'hono'
import { StatusCodes } from 'http-status-codes'

export function containerBridge(
  client: Pryx,
  options: {
    token: string
    image: string
    port: number
    idle: number
    spec?: Omit<MakeRequest, 'image' | 'ports' | 'idle'>
    send?: typeof fetch
  },
) {
  const app = new Hono()
  app.use('*', async (c, next) => {
    if (c.req.header('authorization') !== `Bearer ${options.token}`)
      return c.json({ error: 'unauthorized' }, StatusCodes.UNAUTHORIZED)
    await next()
  })
  app.post('/containers', async (c) => {
    const { cell } = await c.req.json<{ cell: string }>()
    if (typeof cell !== 'string' || !cell || cell.length > 512)
      return c.json({ error: 'invalid cell' }, StatusCodes.UNPROCESSABLE_ENTITY)
    return c.json(
      (
        await client.sandbox({
          ...options.spec,
          idempotencyKey: `celld:${cell}`,
          image: options.image,
          idle: options.idle,
          ports: [{ name: 'http', port: options.port, protocol: 'tcp', audience: 'private' }],
        })
      ).toJSON(),
    )
  })
  app.post('/containers/:id/sleep', async (c) =>
    c.json({ sandbox: await (await client.sandbox(c.req.param('id'))).sleep() }),
  )
  app.delete('/containers/:id', async (c) => c.json(await (await client.sandbox(c.req.param('id'))).delete()))
  app.all('/containers/:id/fetch/*', async (c) => {
    const id = c.req.param('id')
    await (await client.sandbox(id)).wake()
    const endpoint = (await (await client.sandbox(id)).endpoints()).find((p) => p.name === 'http')
    if (!endpoint) return c.json({ error: 'no HTTP port' }, StatusCodes.SERVICE_UNAVAILABLE)
    const host = endpoint.host.includes(':') ? `[${endpoint.host}]` : endpoint.host
    const suffix = c.req.path.split('/fetch/').slice(1).join('/fetch/')
    const url = new URL(`http://${host}:${endpoint.port}/${suffix}`)
    url.search = new URL(c.req.url).search
    const headers = new Headers(c.req.raw.headers)
    for (const key of ['authorization', 'host', 'connection', 'transfer-encoding']) headers.delete(key)
    return (options.send ?? fetch)(url, {
      method: c.req.method,
      headers,
      body: ['GET', 'HEAD'].includes(c.req.method) ? undefined : c.req.raw.body,
      redirect: 'manual',
      signal: c.req.raw.signal,
    })
  })
  return app
}
