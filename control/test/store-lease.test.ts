// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Store-time authority, racing renewals and crash recovery use the real placement ledger, with a conditional store double. */
import { describe, expect, test } from 'bun:test'
import { sql } from 'drizzle-orm'
import { StatusCodes } from 'http-status-codes'
import { SECOND_MS } from '../src/durations.ts'
import { decodeHandle } from '../src/handle.ts'
import { jsonb, one } from '../src/sql.ts'
import { StoreLeaseAuthority } from '../src/store-lease.ts'
import {
  type LeaseBucket,
  type LeaseObject,
  type LeaseRead,
  S3LeaseBucket,
} from '../src/store-lease-bucket.ts'
import { hasDatabase } from './nodes.ts'
import { publicContext } from './public-context.ts'

class Bucket implements LeaseBucket {
  now = 100_000
  version = 0
  object: LeaseObject | null = null
  race = false
  async read(): Promise<LeaseRead> {
    return {
      object: structuredClone(this.object),
      etag: this.version ? String(this.version) : null,
      now: this.now,
    }
  }
  async compare(_key: string, etag: string | null, object: LeaseObject) {
    if (this.race && this.object) {
      this.race = false
      this.object.expiresAtMs = this.now + 30 * SECOND_MS
      this.version++
    }
    if (etag !== (this.version ? String(this.version) : null)) return false
    this.object = structuredClone(object)
    this.version++
    return true
  }
  async links() {
    return { readUrl: 'http://store/lease?read-secret', writeUrl: 'http://store/lease?write-secret' }
  }
}
describe.skipIf(!hasDatabase)('store lease control', () => {
  test('feature-off nodes receive no grant; store time and identity survive a lost grant reply', async () => {
    const c = await publicContext(),
      bucket = new Bucket(),
      calls: unknown[] = []
    let session = 'session'
    let lose = true
    const authority = new StoreLeaseAuthority(
      c.db,
      c.boxes,
      {
        call: (async (_node: unknown, _method: unknown, _path: unknown, options: unknown) => {
          if (_method === 'GET') return { session }
          calls.push(options)
          if (lose) {
            lose = false
            throw new Error('lost grant reply')
          }
        }) as never,
      },
      bucket,
      'test',
      30,
      10,
    )
    try {
      const made = (await (await c.request('/sandboxes', 'POST', { image: 'alpine:3.22' })).json()) as {
          id: string
        },
        row = await c.boxes.record(made.id),
        ref = decodeHandle(row.handle as string)
      await authority.beforeStart(ref.node ?? '', made.id, ref.epoch)
      expect(calls).toHaveLength(0)
      await c.db.execute(sql`UPDATE fleet_nodes SET features=${jsonb(['store-lease'])} WHERE id=${ref.node}`)
      await expect(authority.beforeStart(ref.node ?? '', made.id, ref.epoch)).rejects.toThrow('lost grant')
      expect(bucket.object?.expiresAtMs).toBe(bucket.now + 30 * SECOND_MS)
      const generation = bucket.object?.generation
      await authority.beforeStart(ref.node ?? '', made.id, ref.epoch)
      expect(bucket.version).toBe(1)
      expect(bucket.object?.generation).toBe(generation)
      session = 'another-daemon'
      await expect(authority.beforeStart(ref.node ?? '', made.id, ref.epoch)).rejects.toThrow('authority')
      expect(bucket.version).toBe(1)
      session = 'session'
      const ledger = await one(c.db, sql`SELECT * FROM store_lease_placements WHERE workload=${made.id}`)
      expect(JSON.stringify(ledger)).not.toContain('secret')
      bucket.now += 30 * SECOND_MS
      await expect(authority.beforeStart(ref.node ?? '', made.id, ref.epoch)).rejects.toThrow('new epoch')
    } finally {
      await c.close()
    }
  })
  test('a winning renewal prevents displacement; an expired CAS fence recovers after a crash without losing a node', async () => {
    const c = await publicContext(),
      bucket = new Bucket()
    const authority = new StoreLeaseAuthority(
      c.db,
      c.boxes,
      { call: (async () => ({ session: 'session' })) as never },
      bucket,
      'test',
      30,
      10,
    )
    try {
      const made = (await (await c.request('/sandboxes', 'POST', { image: 'alpine:3.22' })).json()) as {
          id: string
        },
        row = await c.boxes.record(made.id),
        ref = decodeHandle(row.handle as string)
      await c.db.execute(sql`UPDATE fleet_nodes SET features=${jsonb(['store-lease'])} WHERE id=${ref.node}`)
      await authority.beforeStart(ref.node ?? '', made.id, ref.epoch)
      bucket.now += 30 * SECOND_MS
      bucket.race = true
      await authority.tick()
      expect(
        (
          await one<{ state: string }>(
            c.db,
            sql`SELECT state FROM fleet_placements WHERE workload=${made.id}`,
          )
        )?.state,
      ).toBe('placed')
      expect(await one(c.db, sql`SELECT 1 FROM store_lease_fences WHERE workload=${made.id}`)).toBeNull()
      bucket.now += 30 * SECOND_MS
      await authority.tick()
      expect(bucket.object?.generation).toStartWith('fenced:')
      expect(
        (
          await one<{ state: string }>(
            c.db,
            sql`SELECT state FROM fleet_placements WHERE workload=${made.id}`,
          )
        )?.state,
      ).toBe('displaced')
      expect(
        (await one<{ lifecycle: string }>(c.db, sql`SELECT lifecycle FROM fleet_nodes WHERE id=${ref.node}`))
          ?.lifecycle,
      ).toBe('active')
      // Simulate the object-store CAS committing before the database transaction survives a crash.
      await c.db.execute(sql`DELETE FROM store_lease_fences WHERE workload=${made.id}`)
      await c.db.execute(sql`UPDATE fleet_placements SET state='placed' WHERE workload=${made.id}`)
      const version = bucket.version
      await authority.tick()
      expect(bucket.version).toBe(version)
      expect(await one(c.db, sql`SELECT 1 FROM store_lease_fences WHERE workload=${made.id}`)).not.toBeNull()
    } finally {
      await c.close()
    }
  })
  test('malformed expiry cannot authorize a takeover', async () => {
    const c = await publicContext(),
      bucket = new Bucket()
    const authority = new StoreLeaseAuthority(
      c.db,
      c.boxes,
      { call: (async () => ({ session: 'session' })) as never },
      bucket,
      'test',
      30,
      10,
    )
    try {
      const made = (await (await c.request('/sandboxes', 'POST', { image: 'alpine:3.22' })).json()) as {
          id: string
        },
        row = await c.boxes.record(made.id),
        ref = decodeHandle(row.handle as string)
      await c.db.execute(sql`UPDATE fleet_nodes SET features=${jsonb(['store-lease'])} WHERE id=${ref.node}`)
      await authority.beforeStart(ref.node ?? '', made.id, ref.epoch)
      if (bucket.object) bucket.object.expiresAtMs = Number.NaN
      await authority.tick()
      expect(bucket.version).toBe(1)
      expect(await one(c.db, sql`SELECT 1 FROM store_lease_fences WHERE workload=${made.id}`)).toBeNull()
    } finally {
      await c.close()
    }
  })
})
test('presigning leaves a changing If-Match unsigned and uses only the configured runtime endpoint', async () => {
  const bucket = new S3LeaseBucket({
    endpoint: 'https://control-store.test',
    runtimeEndpoint: 'https://node-store.test',
    region: 'auto',
    bucket: 'leases',
    accessKeyId: 'fixture-key',
    secretAccessKey: 'fixture-secret',
  })
  const links = await bucket.links('one')
  expect(new URL(links.writeUrl).hostname).toBe('node-store.test')
  expect(new URL(links.writeUrl).searchParams.get('X-Amz-SignedHeaders')).not.toContain('if-match')
})

test('startup probe rejects a store that ignores conditional writes', async () => {
  let object: string | null = null,
    version = 0,
    ignore = false
  const server = Bun.serve({
    hostname: '127.0.0.1',
    port: 0,
    async fetch(request) {
      const date = new Date().toUTCString()
      if (request.method === 'GET')
        return new Response(object, {
          status: object ? StatusCodes.OK : StatusCodes.NOT_FOUND,
          headers: { date, etag: `"${version}"` },
        })
      if (
        !ignore &&
        ((request.headers.has('if-none-match') && object !== null) ||
          (request.headers.has('if-match') && request.headers.get('if-match') !== `"${version}"`))
      )
        return new Response(null, { status: StatusCodes.PRECONDITION_FAILED })
      object = await request.text()
      version++
      return new Response(null, { status: StatusCodes.OK, headers: { date } })
    },
  })
  const bucket = new S3LeaseBucket({
    endpoint: server.url.toString(),
    region: 'auto',
    bucket: 'test',
    accessKeyId: 'fixture',
    secretAccessKey: 'fixture',
  })
  try {
    await bucket.probe('one')
    object = null
    ignore = true
    await expect(bucket.probe('two')).rejects.toThrow('ignores conditional create')
  } finally {
    server.stop(true)
  }
})
