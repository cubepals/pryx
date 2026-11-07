// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Run the real CLI process through an HTTP listener and real Postgres sandbox intents. */

import { describe, expect, test } from 'bun:test'
import { HOUR_S, MINUTE_S } from '../../control/src/durations.ts'
import type { NodeClient } from '../../control/src/node-client.ts'
import { createPublicApi } from '../../control/src/public-api.ts'
import { hasDatabase } from '../../control/test/nodes.ts'
import { publicContext } from '../../control/test/public-context.ts'
import nodeProtocol from '../../openapi/node-reads.openapi.json'
import nodeWrites from '../../openapi/node-writes.openapi.json'

describe.skipIf(!hasDatabase)('public CLI', () => {
  test('create and exec accept duration strings', async () => {
    const ctx = await publicContext()
    const server = Bun.serve({ hostname: '127.0.0.1', port: 0, fetch: ctx.app.fetch })
    const invoke = async (args: string[]) => {
      const child = Bun.spawn(['bun', 'cli/src/main.ts', 'sandbox', ...args], {
        env: { ...process.env, PRYX_API_URL: server.url.toString(), PRYX_API_KEY: ctx.credential.key },
        stdout: 'pipe',
        stderr: 'pipe',
      })
      const [stdout, stderr, code] = await Promise.all([
        new Response(child.stdout).text(),
        new Response(child.stderr).text(),
        child.exited,
      ])
      expect({ code, stderr }).toEqual({ code: 0, stderr: '' })
      return stdout
    }
    try {
      const { id } = JSON.parse(
        await invoke(['create', '--image', 'alpine:3.22', '--idle', '5m', '--ttl', '2h']),
      ) as { id: string }
      const row = await ctx.boxes.record(id, 'default')
      expect(row.spec).toMatchObject({ idle: 5 * MINUTE_S, ttl: 2 * HOUR_S })
      const peer = ctx.peers.get(id)
      if (!peer) throw new Error('Missing agent peer')
      peer.output.unshift({
        type: 'output',
        stream: 'stdout',
        bytes: Buffer.from('hello\n').toString('base64'),
      })
      expect(await invoke(['exec', id, '--timeout', '30s', '--', 'echo', 'hello'])).toBe('hello\n')
      expect(peer.requests.at(-1)?.body).toMatchObject({
        timeout: nodeProtocol.components.schemas.ExecRequest.properties.timeoutSeconds.default,
      })
      await invoke(['rm', id])
    } finally {
      server.stop(true)
      await ctx.close()
    }
  })
  test('run prints output, returns command status and deletes its sandbox', async () => {
    const ctx = await publicContext()
    const server = Bun.serve({ hostname: '127.0.0.1', port: 0, fetch: ctx.app.fetch })
    try {
      ctx.nodes.once(/^PUT /, async (answer) => {
        const response = answer()
        const peer = [...ctx.peers.values()].at(-1)
        if (!peer) throw new Error('Missing agent peer')
        peer.output.unshift({
          type: 'output',
          stream: 'stdout',
          bytes: Buffer.from('hello\n').toString('base64'),
        })
        peer.result.code = 7
        return response
      })
      const child = Bun.spawn(
        ['bun', 'cli/src/main.ts', 'run', '--image', 'alpine:3.22', '--', 'echo', '--version'],
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
  test('process image refusal retains the field problem in CLI stderr', async () => {
    const ctx = await publicContext()
    const api = createPublicApi({
      db: ctx.db,
      boxes: ctx.boxes,
      nodes: ctx.nodes as unknown as NodeClient,
      domain: 'sandboxes.test',
      defaultIsolation: 'process',
    })
    const server = Bun.serve({ hostname: '127.0.0.1', port: 0, fetch: api.app.fetch })
    try {
      const child = Bun.spawn(
        ['bun', 'cli/src/main.ts', 'run', '--image', 'alpine:3.22', '--', 'echo', 'hello'],
        {
          env: { ...process.env, PRYX_API_URL: server.url.toString(), PRYX_API_KEY: ctx.credential.key },
          stdout: 'pipe',
          stderr: 'pipe',
        },
      )
      const [stdout, stderr, code] = await Promise.all([
        new Response(child.stdout).text(),
        new Response(child.stderr).text(),
        child.exited,
      ])
      expect(code).toBe(1)
      expect(stdout).toBe('')
      expect(stderr).toContain(nodeWrites['x-pryx-process'].imageProblem)
      expect(stderr).toContain('"field":"image"')
      expect(await ctx.boxes.list('default')).toHaveLength(0)
    } finally {
      server.stop(true)
      await ctx.close()
    }
  })
})
