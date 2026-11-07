// SPDX-License-Identifier: FSL-1.1-ALv2
/** Durable outbox signatures, retries, project scope and SSRF validation use real Postgres and
 * an owned receiver. Failures retain deliveries rather than pretending that an event arrived. */
import { expect, test } from 'bun:test'
import { createHmac } from 'node:crypto'
import { sql } from 'drizzle-orm'
import { StatusCodes } from 'http-status-codes'
import { SECOND_MS } from '../src/durations.ts'
import { one } from '../src/sql.ts'
import { deliver, publicAddress } from '../src/webhook-target.ts'
import { Webhooks } from '../src/webhooks.ts'
import { publicContext } from './public-context.ts'

test('webhook destination policy refuses every non-public address class', async () => {
  for (const ip of [
    '127.0.0.1',
    '169.254.169.254',
    '10.0.0.1',
    '172.16.0.1',
    '192.168.1.1',
    '224.0.0.1',
    '0.0.0.0',
    '100.64.0.1',
    '192.0.2.1',
    '::1',
    'fe80::1',
    'fc00::1',
    'ff02::1',
    '::ffff:127.0.0.1',
    '2001:db8::1',
  ])
    expect(publicAddress(ip)).toBe(false)
  expect(publicAddress('1.1.1.1')).toBe(true)
  expect(publicAddress('2606:4700:4700::1111')).toBe(true)
  let hits = 0
  const server = Bun.serve({
    hostname: '127.0.0.1',
    port: 0,
    fetch: () => {
      hits++
      return new Response('no')
    },
  })
  try {
    await expect(deliver(server.url.toString(), '{}', {})).rejects.toThrow('public')
    await expect(deliver('file:///tmp/private', '{}', {})).rejects.toThrow('Invalid')
    expect(hits).toBe(0)
  } finally {
    server.stop(true)
  }
})
test('created and deleted deliveries sign sanitized bytes and retry into dead letters', async () => {
  const c = await publicContext()
  const received: { body: string; headers: Headers }[] = []
  let status = StatusCodes.OK
  const server = Bun.serve({
    hostname: '127.0.0.1',
    port: 0,
    fetch: async (request) => {
      received.push({ body: await request.text(), headers: request.headers })
      return new Response(null, { status })
    },
  })
  const hooks = new Webhooks(c.db, 'operator-seal-key-for-owned-test', ['127.0.0.1'])
  try {
    const hook = await hooks.create('default', { url: server.url.toString(), kinds: ['created', 'deleted'] })
    expect(hook.secret).toStartWith('whsec_')
    expect(JSON.stringify(await hooks.list('default'))).not.toContain(hook.secret)
    const response = await c.request('/sandboxes', 'POST', {
      image: 'alpine:3.22',
      env: { SECRET: 'must-not-arrive' },
      metadata: { secret: 'must-not-arrive' },
    })
    expect(response.status).toBe(StatusCodes.CREATED)
    const box = (await response.json()) as { id: string }
    await c.request(`/sandboxes/${box.id}`, 'DELETE')
    await hooks.tick()
    expect(received).toHaveLength(2)
    expect(received.map((value) => JSON.parse(value.body).event).sort()).toEqual(['created', 'deleted'])
    for (const value of received) {
      const timestamp = value.headers.get('pryx-timestamp')
      expect(Math.abs(Date.now() / SECOND_MS - Number(timestamp))).toBeLessThan(5)
      const expected = createHmac('sha256', hook.secret).update(`${timestamp}.${value.body}`).digest('hex')
      expect(value.headers.get('pryx-signature')).toBe(`v1=${expected}`)
      expect(value.body).not.toContain('must-not-arrive')
      expect(Object.keys(JSON.parse(value.body)).sort()).toEqual(['at', 'event', 'id', 'sandboxId'])
    }
    expect(await hooks.delete('other', hook.id)).toEqual({ existed: false })
    await expect(hooks.deadletters('other', hook.id)).rejects.toMatchObject({ code: 'not_found' })
    status = StatusCodes.SERVICE_UNAVAILABLE
    await c.request('/sandboxes', 'POST', { image: 'alpine:3.22' })
    for (let i = 0; i < 5; i++) {
      await c.db.execute(sql`UPDATE webhook_deliveries SET next_at=now() WHERE delivered_at IS NULL`)
      await hooks.tick()
    }
    const dead = (await hooks.deadletters('default', hook.id)) as { attempts: number }[]
    expect(dead).toHaveLength(1)
    expect(dead[0]?.attempts).toBe(5)
    expect(
      (
        await one<{ count: number }>(
          c.db,
          sql`SELECT count(*)::int AS count FROM webhook_deliveries WHERE delivered_at IS NOT NULL`,
        )
      )?.count,
    ).toBe(2)
    expect(await hooks.delete('default', hook.id)).toEqual({ existed: true })
  } finally {
    server.stop(true)
    await c.close()
  }
})
