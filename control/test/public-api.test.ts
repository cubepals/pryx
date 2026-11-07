// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Public auth, request identities, isolation and lifecycle against real Postgres. */
import { afterAll, beforeAll, describe, expect, test } from 'bun:test'
import { sql } from 'drizzle-orm'
import { StatusCodes } from 'http-status-codes'
import nodeWrites from '../../openapi/node-writes.openapi.json'
import { SECOND_MS } from '../src/durations.ts'
import { SDK_DEFAULTS } from '../src/sdk-defaults.ts'
import { one } from '../src/sql.ts'
import { hasDatabase, report } from './nodes.ts'
import { publicContext } from './public-context.ts'

const spec = {
  image: 'alpine:3.22',
  resources: { memoryMb: 128 },
  ports: [{ name: 'http', port: 8080, audience: 'public' }],
  secrets: { TOKEN: 'never-return-this' },
}

describe.skipIf(!hasDatabase)('public API', () => {
  let ctx: Awaited<ReturnType<typeof publicContext>>
  beforeAll(async () => {
    ctx = await publicContext()
  })
  afterAll(async () => {
    await ctx?.close()
  })

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
    const peer = ctx.peers.get(made.id)
    if (!peer) throw new Error('Missing agent peer')
    peer.output.unshift({
      type: 'output',
      stream: 'stdout',
      bytes: Buffer.from('hello\n').toString('base64'),
    })
    const path = `/sandboxes/${made.id}/exec`,
      body = { command: ['echo', 'hello'], env: { LANG: 'C' } }
    const first = await ctx.request(path, 'POST', body, 'exec-one')
    expect(first.status).toBe(StatusCodes.OK)
    const text = await first.text()
    expect(text).toContain('hello')
    expect(await (await ctx.request(path, 'POST', body, 'exec-one')).text()).toBe(text)
    expect(peer.calls.filter((c) => c === 'POST /_pryx/processes')).toHaveLength(1)
    expect(peer.requests[0]?.body).toMatchObject({ command: ['echo', 'hello'], env: { LANG: 'C' } })
    peer.hooks.start = async () => new Response(null, { status: StatusCodes.INTERNAL_SERVER_ERROR })
    expect((await ctx.request(path, 'POST', body, 'exec-lost')).status).toBe(StatusCodes.CONFLICT)
    expect((await ctx.request(path, 'POST', body, 'exec-lost')).status).toBe(StatusCodes.CONFLICT)
    expect(peer.calls.filter((c) => c === 'POST /_pryx/processes')).toHaveLength(2)
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
  test('container templates do not warm copies on process nodes', async () => {
    const node = await one<{ id: string; features: string[] }>(
      ctx.db,
      sql`SELECT id,features FROM fleet_nodes`,
    )
    if (!node) throw new Error('Missing enrolled node')
    try {
      await ctx.db.execute(
        sql`UPDATE fleet_nodes SET features=${JSON.stringify([...node.features, 'process-tier'])}::jsonb WHERE id=${node.id}`,
      )
      const response = await ctx.request('/templates', 'POST', {
        name: 'container-only',
        spec: { ...spec, isolation: 'container' },
      })
      expect(response.status).toBe(StatusCodes.CREATED)
      expect(((await response.json()) as { copies: unknown[] }).copies).toEqual([])
    } finally {
      await ctx.db.execute(
        sql`UPDATE fleet_nodes SET features=${JSON.stringify(node.features)}::jsonb WHERE id=${node.id}`,
      )
    }
  })
  test('wasm creation retains packages and refuses image and path sources', async () => {
    const node = await one<{ id: string; features: string[] }>(
      ctx.db,
      sql`SELECT id,features FROM fleet_nodes`,
    )
    if (!node) throw new Error('Missing enrolled node')
    let created: string | undefined
    try {
      await ctx.db.execute(
        sql`UPDATE fleet_nodes SET features=${JSON.stringify([...node.features, 'wasm-tier', 'wasm-packages'])}::jsonb WHERE id=${node.id}`,
      )
      const image = await ctx.request('/sandboxes', 'POST', { ...spec, isolation: 'wasm' })
      expect(image.status).toBe(StatusCodes.UNPROCESSABLE_ENTITY)
      expect(await image.json()).toMatchObject({
        error: {
          details: [{ field: 'image', problem: nodeWrites['x-pryx-wasm'].imageProblem }],
        },
      })
      const invalid = await ctx.request('/sandboxes', 'POST', {
        ...spec,
        image: undefined,
        isolation: 'wasm',
        packages: ['../../host'],
      })
      expect(invalid.status).toBe(StatusCodes.UNPROCESSABLE_ENTITY)
      const udp = await ctx.request('/sandboxes', 'POST', {
        isolation: 'wasm',
        ports: [{ name: 'datagram', port: 3000, protocol: 'udp' }],
      })
      expect(udp.status).toBe(StatusCodes.UNPROCESSABLE_ENTITY)
      expect(await udp.json()).toMatchObject({
        error: { details: [{ field: 'ports', problem: nodeWrites['x-pryx-wasm'].portProblem }] },
      })
      const response = await ctx.request('/sandboxes', 'POST', {
        ...spec,
        image: undefined,
        isolation: 'wasm',
        packages: ['coreutils'],
      })
      expect(response.status).toBe(StatusCodes.CREATED)
      const view = (await response.json()) as { id: string; isolation: string }
      created = view.id
      expect(view.isolation).toBe('wasm')
      const stored = await one<{ spec: { packages: string[] } }>(
        ctx.db,
        sql`SELECT spec FROM sandboxes WHERE id=${view.id}`,
      )
      expect(stored?.spec.packages).toEqual(['coreutils'])
      const warmed = await ctx.request('/templates', 'POST', {
        name: 'wasm-base',
        spec: { isolation: 'wasm', packages: ['coreutils'] },
      })
      expect(warmed.status).toBe(StatusCodes.CREATED)
      const template = (await warmed.json()) as { id: string; copies: Array<{ ready: boolean }> }
      expect(template.copies[0]?.ready).toBe(true)
      const forked = await ctx.request('/sandboxes', 'POST', { template: template.id })
      expect(forked.status).toBe(StatusCodes.CREATED)
      const fork = (await forked.json()) as { id: string; isolation: string }
      expect(fork.isolation).toBe('wasm')
      await ctx.request(`/sandboxes/${fork.id}`, 'DELETE')
    } finally {
      if (created) await ctx.request(`/sandboxes/${created}`, 'DELETE')
      await ctx.db.execute(
        sql`UPDATE fleet_nodes SET features=${JSON.stringify(node.features)}::jsonb WHERE id=${node.id}`,
      )
    }
  })
  test('sandbox views retain the node reported process exit', async () => {
    const created = await ctx.request('/sandboxes', 'POST', spec)
    expect(created.status).toBe(StatusCodes.CREATED)
    const made = (await created.json()) as {
      id: string
      node: string
      epoch: number
    }
    try {
      const exit = { code: 37, oomKilled: false, at: new Date().toISOString() }
      await ctx.db.execute(
        sql`INSERT INTO fleet_observations(node_id,workload,epoch,state,report)
            VALUES(${made.node},${made.id},${made.epoch},'running',${JSON.stringify(report(made.id, made.epoch, { exit }))}::jsonb)
            ON CONFLICT(node_id,workload) DO UPDATE SET report=EXCLUDED.report`,
      )
      expect(((await (await ctx.request(`/sandboxes/${made.id}`)).json()) as { exit: unknown }).exit).toEqual(
        exit,
      )
    } finally {
      await ctx.request(`/sandboxes/${made.id}`, 'DELETE')
    }
  })
})

test('v1 file envelopes carry agent bytes without executing a sandbox command', async () => {
  const ctx = await publicContext()
  try {
    const made = (await (await ctx.request('/sandboxes', 'POST', { image: 'alpine:3.22' })).json()) as {
      id: string
    }
    const data = Buffer.alloc(100_000, 42)
    const file = `/sandboxes/${made.id}/files/nested/data`
    expect((await ctx.request(file, 'PUT', { dataBase64: data.toString('base64') })).status).toBe(
      StatusCodes.OK,
    )
    const read = async (path: string, member: string) => {
      const response = await ctx.request(path)
      expect(response.status).toBe(StatusCodes.OK)
      const value = (await response.json()) as { archiveBase64: string }
      const files = await new Bun.Archive(Buffer.from(value.archiveBase64, 'base64')).files()
      const entry = files.get(member)
      if (!entry) throw new Error('Missing public tar member')
      return Buffer.from(await entry.arrayBuffer())
    }
    expect(await read(file, 'nested/data')).toEqual(data)
    const bytes = SDK_DEFAULTS.fileChunkBytes
    const chunk = `/sandboxes/${made.id}/file-chunks/nested/data`
    expect(await read(`${chunk}?offset=${bytes}`, 'data')).toEqual(data.subarray(bytes, 2 * bytes))
    expect(await read(`${chunk}?offset=${Math.ceil(data.length / bytes) * bytes}`, 'data')).toHaveLength(0)
    const first = Buffer.alloc(bytes, 11),
      last = Buffer.from('last')
    expect(
      (await ctx.request(chunk, 'PUT', { offset: 0, dataBase64: first.toString('base64') })).status,
    ).toBe(StatusCodes.OK)
    expect(
      (await ctx.request(chunk, 'PUT', { offset: bytes, dataBase64: last.toString('base64') })).status,
    ).toBe(StatusCodes.OK)
    expect(await read(file, 'nested/data')).toEqual(Buffer.concat([first, last]))
    expect((await ctx.request(`/sandboxes/${made.id}/files/nested%2F..%2Fescape`)).status).toBe(
      StatusCodes.UNPROCESSABLE_ENTITY,
    )
    expect(ctx.nodes.calls.some((c) => c.path.endsWith('/exec') || c.path.endsWith('/exec-stream'))).toBe(
      false,
    )
    expect(ctx.peers.get(made.id)?.calls.some((c) => c === 'POST /_pryx/processes')).toBe(false)
    const peer = ctx.peers.get(made.id)
    if (!peer) throw new Error('Missing agent peer')
    const large = Buffer.alloc(nodeWrites['x-pryx-agent'].maxRequestBytes + 1, 42)
    await fetch(peer.url + '/_pryx/files/content?path=large', {
      method: 'PUT',
      headers: { authorization: `Bearer ${peer.token()}` },
      body: large,
    })
    expect((await ctx.request(file)).status).toBe(StatusCodes.UNPROCESSABLE_ENTITY)
    expect(await read(`${chunk}?offset=${bytes}`, 'data')).toEqual(large.subarray(bytes, 2 * bytes))
    await ctx.request(`/sandboxes/${made.id}`, 'DELETE')
  } finally {
    await ctx.close()
  }
})

test('a long lifecycle response outlives the HTTP idle timer', async () => {
  const ctx = await publicContext()
  try {
    const made = (await (await ctx.request('/sandboxes', 'POST', spec)).json()) as { id: string }
    const project = ctx.boxes.projects
    if (!project) throw new Error('Missing project admission')
    const write = project.write.bind(project)
    let waiting = true
    project.write = async (...args) => {
      if (waiting) {
        waiting = false
        await Bun.sleep(12 * SECOND_MS)
      }
      return write(...args)
    }
    const original = ctx.boxes.delete.bind(ctx.boxes)
    ctx.boxes.delete = async (...args) => {
      await new Promise((resolve) => setTimeout(resolve, 1.2 * SECOND_MS))
      return original(...args)
    }
    const server = Bun.serve({
      hostname: '127.0.0.1',
      port: 0,
      idleTimeout: 1,
      fetch: (request, server) => ctx.app.fetch(request, { beginStream: () => server.timeout(request, 0) }),
    })
    try {
      const result = await fetch(`${server.url}v1/sandboxes/${made.id}`, {
        method: 'DELETE',
        headers: { authorization: `Bearer ${ctx.credential.key}` },
      })
      expect(result.status).toBe(StatusCodes.OK)
      expect(await result.json()).toEqual({ existed: true })
    } finally {
      ctx.boxes.delete = original
      project.write = write
      server.stop(true)
    }
  } finally {
    await ctx.close()
  }
})
