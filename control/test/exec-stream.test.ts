// SPDX-License-Identifier: FSL-1.1-ALv2
/** Live forwarding and durable keyed outcomes against real Postgres; public-api.test.ts also verifies uncertain outcomes. */
import { describe, expect, test } from 'bun:test'
import { StatusCodes } from 'http-status-codes'
import type { ExecEvent } from '../../sdk/typescript/src/index.ts'
import { ndjson } from '../../sdk/typescript/src/transport.ts'
import { hasDatabase } from './nodes.ts'
import { publicContext } from './public-context.ts'

describe.skipIf(!hasDatabase)('live exec forwarding', () => {
  test('agent forwarding emits its first event before command completion', async () => {
    const ctx = await publicContext()
    let finish: () => void = () => {}
    const finished = new Promise<void>((resolve) => {
      finish = resolve
    })
    const _calls = 0
    try {
      const made = await ctx.request('/sandboxes', 'POST', { image: 'alpine:3.22' })
      const { id } = (await made.json()) as { id: string }
      const peer = ctx.peers.get(id)
      if (!peer) throw new Error('Missing agent peer')
      peer.output.unshift({
        type: 'output',
        stream: 'stdout',
        bytes: Buffer.from('first\n').toString('base64'),
      })
      peer.hooks.output = finished
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
      expect(peer.calls.filter((c) => c === 'POST /_pryx/processes')).toHaveLength(1)
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
