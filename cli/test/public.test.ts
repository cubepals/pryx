// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Run the real CLI process through an HTTP listener and real Postgres sandbox intents. */

import { describe, expect, test } from 'bun:test'
import { hasDatabase } from '../../control/test/nodes.ts'
import { publicContext } from '../../control/test/public-context.ts'

describe.skipIf(!hasDatabase)('public CLI', () => {
  test('run prints output, returns command status and deletes its sandbox', async () => {
    const ctx = await publicContext()
    const server = Bun.serve({ hostname: '127.0.0.1', port: 0, fetch: ctx.app.fetch })
    try {
      ctx.nodes.once(/POST .*\/exec$/, async () => ({
        stdout: 'hello\n',
        stderr: '',
        exitCode: 7,
        stdoutTruncated: false,
        stderrTruncated: false,
        timedOut: false,
        killed: false,
        durationMs: 1,
      }))
      const child = Bun.spawn(
        ['bun', 'cli/src/main.ts', 'run', '--image', 'alpine:3.22', '--', 'echo', 'hello'],
        {
          env: { ...process.env, PRYX_API_URL: server.url.toString(), PRYX_API_KEY: ctx.credential.key },
          stdout: 'pipe',
          stderr: 'pipe',
        },
      )
      expect(await new Response(child.stdout).text()).toBe('hello\n')
      expect(await new Response(child.stderr).text()).toBe('')
      expect(await child.exited).toBe(7)
      expect(await ctx.boxes.list('default')).toHaveLength(0)
    } finally {
      server.stop(true)
      await ctx.close()
    }
  })
})
