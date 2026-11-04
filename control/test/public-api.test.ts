// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Public auth, request identities, isolation and lifecycle against real Postgres. */
import { afterAll, beforeAll, describe, expect, test } from 'bun:test'
import { sql } from 'drizzle-orm'
import { StatusCodes } from 'http-status-codes'
import { one } from '../src/sql.ts'
import { hasDatabase } from './nodes.ts'
import { publicContext } from './public-context.ts'

describe.skipIf(!hasDatabase)('public API', () => {
  let ctx: Awaited<ReturnType<typeof publicContext>>
  beforeAll(async () => {
    ctx = await publicContext()
  })
  afterAll(async () => {
    await ctx?.close()
  })
  const spec = {
    image: 'alpine:3.22',
    resources: { memoryMb: 128 },
    ports: [{ name: 'http', port: 8080, audience: 'public' }],
    secrets: { TOKEN: 'never-return-this' },
  }
  test('keys are hashed, project-scoped and revocable', async () => {
    const raw = await one<{ hash: string }>(ctx.db, sql`SELECT hash FROM keys WHERE id=${ctx.credential.id}`)
    expect(raw?.hash).not.toBe(ctx.credential.key)
    expect((await ctx.app.request('http://c/v1/sandboxes')).status).toBe(StatusCodes.UNAUTHORIZED)
    await ctx.db.execute(sql`UPDATE keys SET revoked_at=now() WHERE id=${ctx.credential.id}`)
    expect((await ctx.request('/sandboxes')).status).toBe(StatusCodes.UNAUTHORIZED)
    await ctx.db.execute(sql`UPDATE keys SET revoked_at=NULL WHERE id=${ctx.credential.id}`)
  })
  test('create retries persist their result, reject reuse and never expose secrets', async () => {
    const response = await ctx.request('/sandboxes', 'POST', spec, 'create-one')
    const text = await response.text()
    expect(response.status).toBe(StatusCodes.CREATED)
    expect(text).not.toContain('never-return-this')
    const first = JSON.parse(text)
    expect(first.endpoints[0].hostname).toBe(`${first.id}.sandboxes.test`)
    const retry = await ctx.request('/sandboxes', 'POST', spec, 'create-one')
    expect(await retry.json()).toEqual(first)
    expect((await ctx.request('/sandboxes', 'POST', { ...spec, image: 'other' }, 'create-one')).status).toBe(
      StatusCodes.UNPROCESSABLE_ENTITY,
    )
    expect((await ctx.request(`/sandboxes/${first.id}/sleep`, 'POST')).status).toBe(StatusCodes.OK)
    expect((await ctx.request(`/sandboxes/${first.id}/wake`, 'POST')).status).toBe(StatusCodes.OK)
    const snapshot = (await (await ctx.request(`/sandboxes/${first.id}/snapshots`, 'POST')).json()) as {
      id: string
    }
    const fork = await ctx.request(`/snapshots/${snapshot.id}/sandboxes`, 'POST', {})
    expect(fork.status).toBe(StatusCodes.CREATED)
    const made = (await fork.json()) as { id: string }
    expect((await ctx.request(`/sandboxes/${first.id}/events`)).status).toBe(StatusCodes.OK)
    await ctx.request(`/sandboxes/${first.id}`, 'DELETE')
    await ctx.request(`/sandboxes/${made.id}`, 'DELETE')
  })
  test('exec caches its exact bounded answer and refuses uncertain reexecution', async () => {
    const made = (await (await ctx.request('/sandboxes', 'POST', spec)).json()) as { id: string }
    const result = {
      stdout: 'hello\n',
      stderr: '',
      exitCode: 0,
      stdoutTruncated: false,
      stderrTruncated: false,
      timedOut: false,
      killed: false,
      durationMs: 1,
    }
    ctx.nodes.once(/POST .*\/exec$/, async () => result)
    const path = `/sandboxes/${made.id}/exec`,
      body = { command: ['echo', 'hello'], env: { LANG: 'C' } }
    const first = await ctx.request(path, 'POST', body, 'exec-one')
    expect(first.status).toBe(StatusCodes.OK)
    const text = await first.text()
    expect(text).toContain('hello')
    expect(await (await ctx.request(path, 'POST', body, 'exec-one')).text()).toBe(text)
    expect(ctx.nodes.calls.filter((c) => c.path.endsWith('/exec')).length).toBe(1)
    expect(
      (ctx.nodes.calls.find((c) => c.path.endsWith('/exec'))?.body as { command: string[] }).command.slice(
        0,
        3,
      ),
    ).toEqual(['env', '--', 'LANG=C'])
    ctx.nodes.once(/POST .*\/exec$/, async () => {
      throw new Error('reply lost')
    })
    expect((await ctx.request(path, 'POST', body, 'exec-lost')).status).toBe(StatusCodes.SERVICE_UNAVAILABLE)
    expect((await ctx.request(path, 'POST', body, 'exec-lost')).status).toBe(StatusCodes.CONFLICT)
    expect(
      (await ctx.request(`/sandboxes/${made.id}/files/escape`, 'PUT', { dataBase64: '!bad' })).status,
    ).toBe(StatusCodes.UNPROCESSABLE_ENTITY)
    await ctx.request(`/sandboxes/${made.id}`, 'DELETE')
  })
  test('templates warm a stopped snapshot on an active node', async () => {
    const r = await ctx.request('/templates', 'POST', { name: 'base', spec })
    expect(r.status).toBe(StatusCodes.CREATED)
    const template = (await r.json()) as { id: string; copies: Array<{ ready: boolean }> }
    expect(template.copies[0]?.ready).toBe(true)
    const made = await ctx.request('/sandboxes', 'POST', { template: template.id })
    expect(made.status).toBe(StatusCodes.CREATED)
    await ctx.request(`/sandboxes/${((await made.json()) as { id: string }).id}`, 'DELETE')
  })
})
