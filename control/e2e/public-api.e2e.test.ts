// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Public SDK and compiled CLI through a real daemon, Docker, Postgres and S3; run only in CI. */
import { afterAll, beforeAll, describe, expect, test } from 'bun:test'
import { Pryx } from '@cubepals/pryx'
import { MINUTE_MS } from '../src/durations.ts'
import { deployment, ready } from './deployment.ts'

describe.skipIf(!ready)('public API end to end', () => {
  let d: Awaited<ReturnType<typeof deployment>>, client: Pryx
  beforeAll(async () => {
    d = await deployment()
    client = new Pryx({ baseUrl: d.control.apiUrl, apiKey: d.credential.key })
  }, 2 * MINUTE_MS)
  afterAll(async () => {
    await d?.close()
  })
  test(
    'make, files, exec, logs, snapshot, fork, sleep, wake, rest, delete',
    async () => {
      const made = await client.make(
        {
          image: 'alpine:3.22',
          entrypoint: ['sh', '-c', 'trap "exit 0" TERM; echo started; while :; do sleep 1; done'],
          resources: { memoryMb: 128 },
        },
        crypto.randomUUID(),
      )
      try {
        await client.putFile(made.id, 'message', Buffer.from('kept').toString('base64'))
        const answer = await Array.fromAsync(client.exec(made.id, { command: ['cat', '/data/message'] }))
        expect(answer).toContainEqual(expect.objectContaining({ type: 'stdout', data: 'kept' }))
        expect((await client.getFile(made.id, 'message')).archiveBase64).not.toBe('')
        expect(await Array.fromAsync(client.logs(made.id, { follow: false }))).not.toHaveLength(0)
        const snap = await client.snapshot(made.id),
          fork = await client.fork(snap.id)
        try {
          expect((await client.get(fork.id)).state).toBe('running')
        } finally {
          await client.delete(fork.id)
        }
        await client.action(made.id, 'sleep')
        expect((await client.action(made.id, 'wake')).sandbox.state).toBe('running')
        await client.action(made.id, 'rest')
        expect((await client.action(made.id, 'wake')).tier).toBe('restored')
      } finally {
        await client.delete(made.id)
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
        expect({ exit: await p.exited, stderr }).toEqual({ exit: 0, stderr: '' })
        return stdout
      }
      expect(await invoke(['run', '--image', 'alpine:3.22', '--', 'echo', 'hello'])).toBe('hello\n')
      const made = await client.make({
        image: 'alpine:3.22',
        entrypoint: ['sh', '-c', 'while :; do sleep 1; done'],
        resources: { memoryMb: 128 },
      })
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
