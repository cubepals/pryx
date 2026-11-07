// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Fleet flow assertions registered with their existing database fixture. */

import { expect, test } from 'bun:test'
import { sql } from 'drizzle-orm'
import { StatusCodes } from 'http-status-codes'
import { SECOND_MS } from '../src/durations.ts'
import { decodeHandle } from '../src/handle.ts'
import { NodeRefused } from '../src/node-client.ts'
import { RuntimeFull } from '../src/runtime-model.ts'
import { TEARDOWN_MS, TRANSFER_MS } from '../src/workload-api.ts'
import type { flowContext } from './flow-context.ts'
import type { EnrolledNode } from './nodes.ts'

export function registerFlows1(ctx: ReturnType<typeof flowContext>) {
  test('1. a first start on a node with room: PUT, then start, with its room claimed', async () => {
    const [n] = (await ctx.region('first', 1)) as [EnrolledNode]
    const key = ctx.server()
    const since = ctx.nodes.calls.length
    const { sink, seen } = ctx.recorder()
    const handle = await ctx.runtime.ensureProvisioned(key, { regionKey: 'first' }, ctx.SPEC, sink)
    expect(await ctx.callsSince(since)).toEqual([
      `first-0 PUT /v1/workloads/${key} e1 t1200000`,
      `first-0 POST /v1/workloads/${key}/start e1 t60000`,
    ])
    expect(ctx.nodes.calls[since]?.body).toMatchObject({
      image: 'alpine:3.22',
      restart: { policy: 'on-failure' },
    })
    expect(seen).toEqual(['allocating', 'compute', 'handle e1', 'booting'])
    expect(decodeHandle(handle)).toMatchObject({
      node: n.id,
      nodeName: 'first-0',
      epoch: 1,
      region: 'first',
    })
    expect(await ctx.placement(key)).toMatchObject({
      node_id: n.id,
      epoch: '1',
      state: 'placed',
      desired_power: 'running',
      completing: null,
    })
    expect(await ctx.running(n.id)).toBe(1024)
    expect(await ctx.kinds(key)).toEqual(['placement.created', 'placement.placed'])
    expect(await ctx.history(key)).toEqual([
      { epoch: '1', node_id: n.id, reason: 'placed', end_reason: null },
    ])
    // Its start answered running, so the wait asks nothing; the same spec again starts with no PUT.
    await ctx.runtime.waitRunning(handle, AbortSignal.timeout(SECOND_MS))
    const again = await ctx.runtime.ensureProvisioned(key, { regionKey: 'first' }, ctx.SPEC, ctx.quiet)
    expect(decodeHandle(again).epoch).toBe(1)
    expect(await ctx.callsSince(since + 2)).toEqual([`first-0 POST /v1/workloads/${key}/start e1 t30000`])
  })
  test('2. a first placement its node refuses for room is undone, and its room given back', async () => {
    const [n] = (await ctx.region('refused-put', 1)) as [EnrolledNode]
    const key = ctx.server()
    ctx.nodes.once(/^PUT /, async () => {
      throw new NodeRefused(
        StatusCodes.INSUFFICIENT_STORAGE,
        'insufficient_capacity',
        'not enough memory',
        null,
      )
    })
    const since = ctx.nodes.calls.length
    const made = ctx.runtime.ensureProvisioned(key, { regionKey: 'refused-put' }, ctx.SPEC, ctx.quiet)
    await expect(made).rejects.toBeInstanceOf(RuntimeFull)
    await expect(made).rejects.toThrow('The fleet runtime has no room: refused-put-0: not enough memory')
    expect(await ctx.callsSince(since)).toEqual([`refused-put-0 PUT /v1/workloads/${key} e1 t1200000`])
    expect(await ctx.placement(key)).toBeUndefined()
    expect(await ctx.running(n.id)).toBe(0)
    expect(await ctx.history(key)).toEqual([
      {
        epoch: '1',
        node_id: n.id,
        reason: 'placed',
        end_reason: 'refused: insufficient_capacity',
      },
    ])
    expect(await ctx.kinds(key)).toEqual(['placement.created', 'placement.abandoned'])
    expect((await ctx.eventData('placement.abandoned', key)).map((e) => e.data)).toEqual([
      { code: 'insufficient_capacity' },
    ])
  })
  test('3. a start its node refuses for room gives the claim back and moves the server to start', async () => {
    const [a, b] = (await ctx.region('refused-start', 2)) as [EnrolledNode, EnrolledNode]
    const key = ctx.server()
    const first = await ctx.runtime.ensureProvisioned(
      key,
      { regionKey: 'refused-start' },
      ctx.SPEC,
      ctx.quiet,
    )
    const home = decodeHandle(first).node === a.id ? a : b
    const away = home === a ? b : a
    const homeName = await ctx.nameOf(home.id)
    const awayName = await ctx.nameOf(away.id)
    await ctx.runtime.stop(first)
    ctx.nodes.once(/^POST .*\/start$/, async () => {
      throw new NodeRefused(
        StatusCodes.INSUFFICIENT_STORAGE,
        'insufficient_capacity',
        'a copy runs here',
        null,
      )
    })
    const since = ctx.nodes.calls.length
    const after = await ctx.lastEvent()
    const { sink, seen } = ctx.recorder()
    const moved = await ctx.runtime.ensureProvisioned(key, { regionKey: 'refused-start' }, ctx.SPEC, sink)
    expect(await ctx.callsSince(since)).toEqual([
      `${homeName} POST /v1/workloads/${key}/start e1 t30000`,
      `${homeName} POST /v1/workloads/${key}/export e1 t${TRANSFER_MS}`,
      `${awayName} PUT /v1/workloads/${key} e2 t1200000`,
      `${awayName} POST /v1/workloads/${key}/restore e2 t${TRANSFER_MS}`,
      `${homeName} POST /v1/workloads/${key}/fence e- t${TEARDOWN_MS}`,
      `${homeName} DELETE /v1/workloads/${key}?data=delete e2 t${TEARDOWN_MS}`,
      `${awayName} POST /v1/workloads/${key}/start e2 t60000`,
    ])
    expect(seen).toEqual(['allocating', 'handle e1', 'booting', 'storage', 'compute', 'handle e2', 'booting'])
    expect(decodeHandle(moved)).toMatchObject({ node: away.id, epoch: 2 })
    expect(await ctx.running(home.id)).toBe(0)
    expect(await ctx.running(away.id)).toBe(1024)
    expect(await ctx.kinds(key, after)).toEqual([
      'move.exported',
      'placement.rehomed',
      'placement.moved_for_room',
      'placement.placed',
      'placement.restored',
      'copy.fenced',
    ])
    expect((await ctx.eventData('placement.moved_for_room', key)).map((e) => e.data)).toEqual([
      {
        fromNode: home.id,
        fromEpoch: 1,
        why: `${homeName}: a copy runs here`,
      },
    ])
    expect(await ctx.archives(key)).toMatchObject([{ purpose: 'move', status: 'deleted', object_key: null }])
    expect(ctx.stored(key)).toEqual([])
  })
  test('4. start() on a node with no room now claims nothing and asks no node', async () => {
    const [n] = (await ctx.region('full-start', 1)) as [EnrolledNode]
    const key = ctx.server()
    const handle = await ctx.runtime.ensureProvisioned(key, { regionKey: 'full-start' }, ctx.SPEC, ctx.quiet)
    await ctx.runtime.stop(handle)
    await ctx.runtime.ensureProvisioned(
      ctx.server(),
      { regionKey: 'full-start' },
      { ...ctx.SPEC, resources: { memoryMb: 3584 } },
      ctx.quiet,
    )
    const since = ctx.nodes.calls.length
    const started = ctx.runtime.start(handle)
    await expect(started).rejects.toBeInstanceOf(RuntimeFull)
    await expect(started).rejects.toThrow(/^The fleet runtime has no room: full-start-0: /)
    expect(await ctx.callsSince(since)).toEqual([])
    expect((await ctx.placement(key))?.desired_power).toBe('stopped')
    expect(await ctx.running(n.id)).toBe(3584)
  })
  test('5. an upload is tried five times, and one nobody finished is taken over after three hours', async () => {
    const [n] = (await ctx.region('attempts', 1)) as [EnrolledNode]
    const key = ctx.server()
    const fail = () =>
      ctx.nodes.once(/\/upload$/, async () => {
        throw new Error('the store said no')
      })
    // A fourth try that fails leaves it to be sent again.
    const fourth = await ctx.snapshotOn(n.id, key, null, { attempts: 3 })
    fail()
    expect(await ctx.runtime.upload(fourth.id)).toBe('failed')
    // The fifth is final.
    const fifth = await ctx.snapshotOn(n.id, key, null, { attempts: 4 })
    fail()
    expect(await ctx.runtime.upload(fifth.id)).toBe('failed')
    const byId = new Map((await ctx.archives(key)).map((a) => [a.id, a]))
    expect(byId.get(fourth.id)).toMatchObject({
      status: 'local',
      attempts: 4,
      error: 'the store said no',
    })
    expect(byId.get(fifth.id)).toMatchObject({
      status: 'failed',
      attempts: 5,
      error: 'the store said no',
    })
    expect((await ctx.eventData('snapshot.upload_failed', key)).map((e) => e.data)).toEqual([
      { id: fifth.id, error: 'the store said no' },
    ])
    // An upload under way is left to finish; one not heard from in three hours is taken over.
    const fresh = await ctx.snapshotOn(n.id, key, null)
    await ctx.db.execute(sql`UPDATE fleet_archives SET status = 'uploading', upload_started_at = now() - interval '179 minutes'
                         WHERE id = ${fresh.id}`)
    const stale = await ctx.snapshotOn(n.id, key, null)
    await ctx.db.execute(sql`UPDATE fleet_archives SET status = 'uploading', upload_started_at = now() - interval '181 minutes'
                         WHERE id = ${stale.id}`)
    const since = ctx.nodes.calls.length
    expect(await ctx.runtime.upload(fresh.id)).toBe('skipped')
    expect(await ctx.runtime.upload(stale.id)).toBe('ready')
    expect(await ctx.callsSince(since)).toEqual([
      `attempts-0 POST /v1/workloads/${key}/snapshots/${stale.id}/upload e- t${TRANSFER_MS}`,
    ])
    expect((await ctx.archives(key)).find((a) => a.id === stale.id)).toMatchObject({
      status: 'ready',
      attempts: 1,
      object_key: `fleet/test/${key}/${stale.id}.tar.gz`,
    })
  })
  test("6. a node's copy is trimmed only when a newer one of its server reached the store", async () => {
    const [n] = (await ctx.region('trim', 1)) as [EnrolledNode]
    const key = ctx.server()
    const oldest = await ctx.snapshotOn(n.id, key, `fleet/test/${key}/oldest`, {
      capturedAgo: '3 hours',
    })
    const newer = await ctx.snapshotOn(n.id, key, `fleet/test/${key}/newer`, {
      capturedAgo: '2 hours',
    })
    const newest = await ctx.snapshotOn(n.id, key, null, {
      capturedAgo: '1 hour',
    })
    const alone = await ctx.snapshotOn(n.id, ctx.server(), `fleet/test/alone`, {
      capturedAgo: '4 hours',
    })
    const since = ctx.nodes.calls.length
    expect(await ctx.runtime.trimLocalCopies()).toBe(1)
    expect(await ctx.callsSince(since)).toEqual([
      `trim-0 DELETE /v1/workloads/${key}/snapshots/${oldest.id} e- t${TEARDOWN_MS}`,
    ])
    const byId = new Map((await ctx.archives(key)).map((a) => [a.id, a]))
    expect(byId.get(oldest.id)?.local_deleted_at).not.toBeNull()
    expect(byId.get(oldest.id)).toMatchObject({
      status: 'ready',
      object_key: `fleet/test/${key}/oldest`,
    })
    expect(byId.get(newer.id)?.local_deleted_at).toBeNull()
    expect(byId.get(newest.id)?.local_deleted_at).toBeNull()
    expect(ctx.store.objects.has(`fleet/test/${key}/oldest`)).toBe(true)
    expect(alone.id).toBeTruthy()
    expect((await ctx.eventData('snapshot.local_trimmed', key)).map((e) => e.data)).toEqual([
      { id: oldest.id },
    ])
  })
}
