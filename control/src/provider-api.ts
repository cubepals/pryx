// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Operator-only provider operations; neither public keys nor provider identities authorize host changes. */
import type { Hono } from 'hono'
import { StatusCodes } from 'http-status-codes'
import { z } from 'zod'
import type { Autoscale } from './autoscale.ts'
import { OPERATOR_BASE } from './operator-api.ts'
import type { ProviderLedger } from './provider-ledger.ts'
export function withProviders(app: Hono, hosts: ProviderLedger, scaling: Autoscale) {
  app.get(`${OPERATOR_BASE}/providers/:provider/shapes`, async (c) =>
    c.json({
      shapes: await hosts.provider(c.req.param('provider')).shapes(c.req.query('region') ?? 'default'),
    }),
  )
  app.post(`${OPERATOR_BASE}/nodes/acquire`, async (c) => {
    const body = z
      .object({ provider: z.string(), region: z.string(), shape: z.string() })
      .strict()
      .parse(await c.req.json())
    const key = c.req.header('idempotency-key')
    if (!key)
      return c.json(
        { error: { code: 'invalid_request', message: 'Idempotency-Key required' } },
        StatusCodes.UNPROCESSABLE_ENTITY,
      )
    return c.json(await hosts.acquire(body.region, body.provider, body.shape, key), StatusCodes.ACCEPTED)
  })
  app.post(`${OPERATOR_BASE}/acquisitions/:id/cancel`, async (c) =>
    c.json(await hosts.cancel(c.req.param('id'))),
  )
  app.get(`${OPERATOR_BASE}/acquisitions/:id`, async (c) => c.json(await hosts.view(c.req.param('id'))))
  app.post(`${OPERATOR_BASE}/nodes/:id/release`, async (c) =>
    c.json(await scaling.release(c.req.param('id'))),
  )
}
