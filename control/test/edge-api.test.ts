// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Route acknowledgments and session activity over real Postgres, authenticated with the separate edge token. */
import { afterAll, beforeAll, describe, expect, test } from 'bun:test'
import { sql } from 'drizzle-orm'
import { StatusCodes } from 'http-status-codes'
import { edgeApi } from '../src/edge-api.ts'
import { EdgeService } from '../src/edge-service.ts'
import { one } from '../src/sql.ts'
import { hasDatabase } from './nodes.ts'
import { publicContext } from './public-context.ts'

describe.skipIf(!hasDatabase)('edge control', () => {
  let ctx: Awaited<ReturnType<typeof publicContext>>, service: EdgeService
  const token = 'edge-token-for-real-postgres-tests'
  beforeAll(async () => {
    ctx = await publicContext()
    service = new EdgeService(ctx.db, ctx.boxes, 'sandboxes.test')
  })
  afterAll(async () => {
    await ctx?.close()
  })
  test('running waits for the counted edge to acknowledge the route, then sleep routes nowhere', async () => {
    const app = edgeApi(service, token)
    const ask = (path: string, held?: number) =>
      app.request(`http://c${path}`, {
        headers: {
          authorization: `Bearer ${token}`,
          'x-edge-id': 'edge',
          ...(held === undefined ? {} : { 'x-routes-held': String(held) }),
        },
      })
    expect((await app.request('http://c/routes')).status).toBe(StatusCodes.UNAUTHORIZED)
    await ask('/routes')
    const made = (await (
      await ctx.request('/sandboxes', 'POST', {
        image: 'alpine:3.22',
        ports: [{ name: 'http', port: 8080, audience: 'public' }],
      })
    ).json()) as { id: string; state: string }
    expect(made.state).toBe('starting')
    const list = await service.routes()
    expect(list.routes[0]?.state).toBeUndefined()
    expect((await ctx.model.view(made.id, 'default')).state).toBe('starting')
    await ask('/routes', list.version)
    expect((await ctx.model.view(made.id, 'default')).state).toBe('running')
    await ctx.boxes.sleep(made.id)
    expect((await service.routes()).routes[0]?.destination).toBe('0.0.0.0:0')
    expect((await service.wake(`${made.id}:http`)).outcome).toBe('ready')
    const current = await ctx.boxes.get(made.id)
    await service.session('edge', {
      id: 'connection',
      sandbox: made.id,
      epoch: current.epoch ?? 0,
      event: 'connect',
    })
    expect(await one(ctx.db, sql`SELECT 1 FROM sandbox_sessions WHERE sandbox=${made.id}`)).not.toBeNull()
    await service.session('edge', {
      id: 'connection',
      sandbox: made.id,
      epoch: current.epoch ?? 0,
      event: 'disconnect',
    })
    expect(await one(ctx.db, sql`SELECT 1 FROM sandbox_sessions WHERE sandbox=${made.id}`)).toBeNull()
    await ctx.boxes.delete(made.id)
  })
  test('raw port reservations stay quarantined after delete and host collisions are refused', async () => {
    const spec = {
      image: 'alpine:3.22',
      ports: [{ name: 'raw', port: 9090, audience: 'public', front: 'port' }],
    }
    const first = (await (await ctx.request('/sandboxes', 'POST', spec)).json()) as {
      id: string
      endpoints: Array<{ publicPort: number }>
    }
    await ctx.boxes.delete(first.id)
    const second = (await (await ctx.request('/sandboxes', 'POST', spec)).json()) as typeof first
    expect(second.endpoints[0]?.publicPort).toBeGreaterThan(first.endpoints[0]?.publicPort ?? 0)
    const bad = await ctx.request('/sandboxes', 'POST', {
      image: 'alpine:3.22',
      ports: [
        { name: 'one', port: 8080, audience: 'public' },
        { name: 'two', port: 8081, audience: 'public' },
      ],
    })
    expect(bad.status).toBe(StatusCodes.UNPROCESSABLE_ENTITY)
    await ctx.boxes.delete(second.id)
  })
})
