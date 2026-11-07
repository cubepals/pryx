// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Public SDK and compiled CLI through a real daemon, Docker, Postgres and S3; run only in CI. */
import { afterAll, beforeAll, describe, expect, test } from 'bun:test'
import { sql } from 'drizzle-orm'
import {
  type ExecEvent,
  ndjson,
  type Sandbox as SandboxView,
  type Snapshot as SnapshotView,
  Transport,
} from '../../sdk/typescript/src/transport.ts'
import { MINUTE_MS } from '../src/durations.ts'
import { rows } from '../src/sql.ts'
import type { LogRecord } from '../src/wire.ts'
import { deployment, ready } from './deployment.ts'
import { deploymentDiagnostics } from './diagnostics.ts'

describe.skipIf(!ready)('public API end to end', () => {
  let d: Awaited<ReturnType<typeof deployment>>, client: Transport
  beforeAll(async () => {
    d = await deployment()
    const wake = d.control.sandboxes.wake.bind(d.control.sandboxes)
    d.control.sandboxes.wake = async (...args) => {
      try {
        return await wake(...args)
      } catch (error) {
        console.error(
          'OWNED_WAKE_FAILURE',
          error instanceof Error ? { name: error.name, message: error.message } : String(error),
        )
        throw error
      }
    }
    client = new Transport({ url: d.control.apiUrl, key: d.credential.key })
  }, 2 * MINUTE_MS)
  afterAll(async () => {
    await d?.close()
  })
  test(
    'make, files, exec, logs, snapshot, fork, sleep, wake, rest, delete',
    async () => {
      const made = await client.request<SandboxView>(
        '/v1/sandboxes',
        'POST',
        {
          image: 'alpine:3.22',
          entrypoint: ['sh', '-c', 'trap "exit 0" TERM; echo started; while :; do sleep 1; done'],
          resources: { memoryMb: 128 },
        },
        crypto.randomUUID(),
      )
      try {
        await client.request<unknown>(
          `/v1/sandboxes/${encodeURIComponent(made.id)}/files/${encodeURIComponent('message')}`,
          'PUT',
          { dataBase64: Buffer.from('kept').toString('base64') },
        )
        const answer = await Array.fromAsync(
          ndjson<ExecEvent>(
            await client.raw(
              `/v1/sandboxes/${encodeURIComponent(made.id)}/exec`,
              'POST',
              { command: ['cat', '/data/message'] },
              crypto.randomUUID(),
            ),
          ),
        )
        expect(answer).toContainEqual(expect.objectContaining({ type: 'stdout', data: 'kept' }))
        expect(
          (
            await client.request<{ archiveBase64: string }>(
              `/v1/sandboxes/${encodeURIComponent(made.id)}/files/${encodeURIComponent('message')}`,
              'GET',
            )
          ).archiveBase64,
        ).not.toBe('')
        expect(
          await Array.fromAsync(
            ndjson<LogRecord>(
              await client.raw(
                `/v1/sandboxes/${encodeURIComponent(made.id)}/logs?${new URLSearchParams(Object.entries({ follow: false }).map(([k, v]) => [k, String(v)]))}`,
              ),
            ),
          ),
        ).not.toHaveLength(0)
        const snap = await client.request<SnapshotView>(
            `/v1/sandboxes/${encodeURIComponent(made.id)}/snapshots`,
            'POST',
          ),
          fork = await client.request<SandboxView>(
            `/v1/snapshots/${encodeURIComponent(snap.id)}/sandboxes`,
            'POST',
            {},
            crypto.randomUUID(),
          )
        try {
          expect(
            (await client.request<SandboxView>(`/v1/sandboxes/${encodeURIComponent(fork.id)}`, 'GET')).state,
          ).toBe('running')
        } finally {
          await client.request<{ existed: boolean }>(`/v1/sandboxes/${encodeURIComponent(fork.id)}`, 'DELETE')
        }
        await client.request<{ sandbox: SandboxView; tier?: string }>(
          `/v1/sandboxes/${encodeURIComponent(made.id)}/${'sleep'}`,
          'POST',
        )
        expect(
          (
            await client.request<{ sandbox: SandboxView; tier?: string }>(
              `/v1/sandboxes/${encodeURIComponent(made.id)}/${'wake'}`,
              'POST',
            )
          ).sandbox.state,
        ).toBe('running')
        await client.request<{ sandbox: SandboxView; tier?: string }>(
          `/v1/sandboxes/${encodeURIComponent(made.id)}/${'rest'}`,
          'POST',
        )
        expect(
          (
            await client.request<{ sandbox: SandboxView; tier?: string }>(
              `/v1/sandboxes/${encodeURIComponent(made.id)}/${'wake'}`,
              'POST',
            )
          ).tier,
        ).toBe('restored')
      } finally {
        await client.request<{ existed: boolean }>(`/v1/sandboxes/${encodeURIComponent(made.id)}`, 'DELETE')
      }
    },
    3 * MINUTE_MS,
  )
  test(
    'CLI run and every sandbox command use the same control',
    async () => {
      const cli = new URL('../../dist/pryx', import.meta.url).pathname
      const build = Bun.spawn(['bun', 'build', '--compile', 'cli/src/main.ts', '--outfile', cli], {
        stdout: 'ignore',
        stderr: 'pipe',
      })
      expect(await build.exited).toBe(0)
      const invoke = async (args: string[]) => {
        const p = Bun.spawn([cli, ...args], { env: d.env, stdout: 'pipe', stderr: 'pipe' })
        const stdout = await new Response(p.stdout).text(),
          stderr = await new Response(p.stderr).text()
        const exit = await p.exited
        if (exit !== 0) {
          deploymentDiagnostics(d)
          console.error(
            'CLI_LIFECYCLE_REFUSAL',
            args[0],
            args[1],
            await rows(
              d.control.db,
              sql`SELECT kind,data FROM fleet_events WHERE kind IN ('placement.refused','workload.failed') ORDER BY id DESC LIMIT 5`,
            ),
          )
        }
        expect({ exit, stderr }).toEqual({ exit: 0, stderr: '' })
        return stdout
      }
      expect(await invoke(['run', '--image', 'alpine:3.22', '--', 'echo', 'hello'])).toBe('hello\n')
      const made = await client.request<SandboxView>(
        '/v1/sandboxes',
        'POST',
        {
          image: 'alpine:3.22',
          entrypoint: ['sh', '-c', 'trap "exit 0" TERM; while :; do sleep 1; done'],
          resources: { memoryMb: 128 },
        },
        crypto.randomUUID(),
      )
      for (const verb of ['ls', 'get', 'logs', 'sleep', 'wake', 'rest', 'wake'])
        await invoke(['sandbox', verb, ...(verb === 'ls' ? [] : [made.id])])
      const snap = JSON.parse(await invoke(['sandbox', 'snapshot', made.id])) as { id: string }
      const fork = JSON.parse(await invoke(['sandbox', 'fork', snap.id])) as { id: string }
      await invoke(['sandbox', 'exec', fork.id, '--', 'echo', 'cli'])
      await invoke(['sandbox', 'rm', fork.id])
      await invoke(['sandbox', 'rm', made.id])
      for (const verb of ['ls', 'token', 'events']) await invoke(['node', verb])
    },
    4 * MINUTE_MS,
  )
})
