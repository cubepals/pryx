// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Edge-only authenticated routes mounted beside the public API on the private control listener. */
import { timingSafeEqual } from 'node:crypto'
import { Hono } from 'hono'
import { StatusCodes } from 'http-status-codes'
import { z } from 'zod'
import {
  EDGE_FEATURES_HEADER,
  EDGE_ID_HEADER,
  ROUTES_HELD_HEADER,
  ROUTES_VERSION_HEADER,
} from './edge-headers.ts'
import { sawEdge } from './edge-routed.ts'
import { IdleHint, SessionEvent, VisitReports, WakeRequest } from './edge-schemas.ts'
import type { EdgeService } from './edge-service.ts'
import { event } from './sql.ts'
export function edgeApi(service: EdgeService, token?: string) {
  const app = new Hono<{ Variables: { edge: string } }>()
  app.use('*', async (c, next) => {
    const expected = Buffer.from(`Bearer ${token ?? ''}`),
      given = Buffer.from(c.req.header('authorization') ?? '')
    if (!token || expected.length !== given.length || !timingSafeEqual(expected, given))
      return c.json(
        { error: { code: 'unauthorized', message: 'Edge token required' } },
        StatusCodes.UNAUTHORIZED,
      )
    const id = c.req.header(EDGE_ID_HEADER)
    if (!id || id.length > 128)
      return c.json(
        { error: { code: 'invalid_request', message: 'x-edge-id is required (at most 128 characters)' } },
        StatusCodes.UNPROCESSABLE_ENTITY,
      )
    c.set('edge', id)
    await next()
  })
  app.onError((error, c) =>
    c.json(
      {
        error: {
          code: error instanceof z.ZodError ? 'invalid_request' : 'unavailable',
          message: 'Edge request refused',
        },
      },
      error instanceof z.ZodError ? StatusCodes.UNPROCESSABLE_ENTITY : StatusCodes.SERVICE_UNAVAILABLE,
    ),
  )
  app.get('/routes', async (c) => {
    const held = c.req.header(ROUTES_HELD_HEADER),
      version = held === undefined ? null : Number(held)
    if (version !== null && (!Number.isSafeInteger(version) || version < 0))
      return c.json(
        { error: { code: 'invalid_request', message: 'Invalid route version' } },
        StatusCodes.UNPROCESSABLE_ENTITY,
      )
    await sawEdge(service.db, c.get('edge'), version)
    const list = await service.routes(c.req.header(EDGE_FEATURES_HEADER)?.split(',').includes('agent'))
    c.header(ROUTES_VERSION_HEADER, String(list.version))
    c.header('etag', `"${list.digest}"`)
    if (c.req.header('if-none-match') === `"${list.digest}"`) return c.body(null, StatusCodes.NOT_MODIFIED)
    return c.json({ routes: list.routes })
  })
  app.post('/wake', async (c) => c.json(await service.wake(WakeRequest.parse(await c.req.json()).route)))
  app.post('/ping-wake', async (c) => {
    const route = await service.route(WakeRequest.parse(await c.req.json()).route)
    if (route?.state) service.begin(route)
    return c.json({ waking: Boolean(route?.state) })
  })
  app.post('/sessions', async (c) => {
    await service.session(c.get('edge'), SessionEvent.parse(await c.req.json()))
    return c.body(null, StatusCodes.NO_CONTENT)
  })
  app.post('/idle', async (c) => {
    await service.idle(IdleHint.parse(await c.req.json()).sandbox)
    return c.body(null, StatusCodes.NO_CONTENT)
  })
  app.post('/visits', async (c) => {
    for (const r of VisitReports.parse(await c.req.json()).reports)
      await event(service.db, `edge.${r.event}`, {
        workload: r.sandbox,
        data: { bytesUp: r.bytesUp, bytesDown: r.bytesDown },
      })
    return c.body(null, StatusCodes.NO_CONTENT)
  })
  return app
}
