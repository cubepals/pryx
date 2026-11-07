// SPDX-License-Identifier: FSL-1.1-ALv2
/** A newly enrolled node can serve its first sandbox before the periodic registry cache refresh. Real Postgres, no Docker. */
import { describe, expect, test } from 'bun:test'
import { sql } from 'drizzle-orm'
import { StatusCodes } from 'http-status-codes'
import { decodeHandle } from '../src/handle.ts'
import { heartbeat } from '../src/registry.ts'
import { beat, enrollNode, hasDatabase, registryOptions, testCa } from './nodes.ts'
import { publicContext } from './public-context.ts'

describe.skipIf(!hasDatabase)('new node I/O', () => {
  test('first exec does not wait for the cache timer', async () => {
    const ctx = await publicContext()
    try {
      await ctx.db.execute(sql`UPDATE fleet_nodes SET lifecycle='draining'`)
      const node = await enrollNode(ctx.db, await testCa(), 'new-node', { region: 'default' })
      await heartbeat(
        ctx.db,
        registryOptions(),
        node.id,
        node.sha,
        beat(node, 'new-session', 1, [], {
          features: [
            'placement-epochs',
            'data-transfer',
            'local-snapshots',
            'export-exclude',
            'execution-lease',
          ],
        }),
      )
      const response = await ctx.request('/sandboxes', 'POST', {
        image: 'alpine:3.22',
        resources: { memoryMb: 128 },
      })
      expect(response.status).toBe(StatusCodes.CREATED)
      const { id } = (await response.json()) as { id: string }
      expect(decodeHandle((await ctx.boxes.record(id, 'default')).handle as string).node).toBe(node.id)
      ctx.nodes.once(/POST .*\/exec$/, async () => ({
        stdout: 'first\n',
        stderr: '',
        exitCode: 0,
        stdoutTruncated: false,
        stderrTruncated: false,
        timedOut: false,
        killed: false,
        durationMs: 1,
      }))
      const executed = await ctx.request(`/sandboxes/${id}/exec`, 'POST', { command: ['echo', 'first'] })
      expect(executed.status).toBe(StatusCodes.OK)
      expect(await executed.text()).toContain('first')
      await ctx.request(`/sandboxes/${id}`, 'DELETE')
    } finally {
      await ctx.close()
    }
  })
})
