// SPDX-License-Identifier: FSL-1.1-ALv2
/** Live forwarding and durable keyed outcomes against real Postgres; older nodes remain covered by public-api.test.ts. */
import { describe, expect, test } from 'bun:test'
import { sql } from 'drizzle-orm'
import { StatusCodes } from 'http-status-codes'
import { type ExecEvent, ndjson } from '../../sdk/typescript/src/index.ts'
import type { CallOptions, NodeAddress } from '../src/node-client.ts'
import { hasDatabase } from './nodes.ts'
import { publicContext } from './public-context.ts'

describe.skipIf(!hasDatabase)('live exec forwarding', () => {
  test('only an announced feature streams and the first event precedes command completion', async () => {
    const ctx = await publicContext()
    let finish: () => void = () => {}
    const finished = new Promise<void>((resolve) => {
      finish = resolve
    })
    let calls = 0
    try {
      await ctx.db.execute(sql`UPDATE fleet_nodes SET features=features || '["exec-stream"]'::jsonb`)
      await ctx.boxes.engine.refreshRegistry()
      Object.assign(ctx.nodes, {
        async *lines(
          _node: NodeAddress,
          path: string,
          _signal: AbortSignal,
          options: CallOptions & { method?: string },
        ) {
          calls++
          expect(path).toEndWith('/exec-stream')
          expect(options.method).toBe('POST')
          expect(options.epoch).toBeNumber()
          expect(options.headers?.['Idempotency-Key']).toBeString()
          yield JSON.stringify({ type: 'stdout', data: 'first\n', truncated: false })
          await finished
          yield JSON.stringify({ type: 'exit', code: 0, timedOut: false, killed: false, durationMs: 1 })
        },
      })
      const made = await ctx.request('/sandboxes', 'POST', { image: 'alpine:3.22' })
      const { id } = (await made.json()) as { id: string }
      const body = { command: ['echo', 'first'] }
      const response = await ctx.request(`/sandboxes/${id}/exec`, 'POST', body, 'live-one')
      const reader = ndjson<ExecEvent>(response)
      expect((await reader.next()).value).toMatchObject({ type: 'stdout', data: 'first\n' })
      expect((await ctx.request(`/sandboxes/${id}/exec`, 'POST', body, 'live-one')).status).toBe(
        StatusCodes.CONFLICT,
      )
      finish()
      expect(await Array.fromAsync(reader)).toEqual([
        { type: 'exit', code: 0, timedOut: false, killed: false, durationMs: 1 },
      ])
      expect(await (await ctx.request(`/sandboxes/${id}/exec`, 'POST', body, 'live-one')).text()).toContain(
        'first',
      )
      expect(calls).toBe(1)
      expect(
        (await ctx.request(`/sandboxes/${id}/exec`, 'POST', { command: ['other'] }, 'live-one')).status,
      ).toBe(StatusCodes.UNPROCESSABLE_ENTITY)
      await ctx.request(`/sandboxes/${id}`, 'DELETE')
    } finally {
      finish()
      await ctx.close()
    }
  })
})
