// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Fleet flow assertions registered with their existing database fixture. */

import { expect, test } from 'bun:test'
import { sql } from 'drizzle-orm'
import { StatusCodes } from 'http-status-codes'
import { SECOND_MS } from '../src/durations.ts'
import { decodeHandle } from '../src/handle.ts'
import { NodeRefused, NodeUnreachable } from '../src/node-client.ts'
import { drain } from '../src/registry.ts'
import { RuntimeFull } from '../src/runtime-model.ts'
import { rows } from '../src/sql.ts'
import { TEARDOWN_MS, TRANSFER_MS } from '../src/workload-api.ts'
import type { flowContext } from './flow-context.ts'
import type { EnrolledNode } from './nodes.ts'

export function registerFlows2(ctx: ReturnType<typeof flowContext>) {
  test('7. a snapshot, its upload, its deletion, and what upkeep deletes once the store answers', async () => {
    await ctx.region('backup', 1)
    const key = ctx.server()
    const handle = await ctx.runtime.ensureProvisioned(key, { regionKey: 'backup' }, ctx.SPEC, ctx.quiet)
    const since = ctx.nodes.calls.length
    const taken = await ctx.runtime.snapshot(handle)
    // Its upload runs on its own once the snapshot is taken.
    for (let i = 0; i < 100 && (await ctx.archives(key))[0]?.status !== 'ready'; i++)
      await new Promise((resolve) => setTimeout(resolve, 0.02 * SECOND_MS))
    const [row] = await ctx.archives(key)
    if (row === undefined) throw new Error('no archive')
    expect(taken.sizeBytes).toBe(5)
    expect(await ctx.callsSince(since)).toEqual([
      `backup-0 GET /v1/workloads/${key} e- t-`,
      `backup-0 POST /v1/workloads/${key}/snapshots e1 t${TRANSFER_MS}`,
      `backup-0 POST /v1/workloads/${key}/snapshots/local-${row.id}/upload e- t${TRANSFER_MS}`,
    ])
    expect(ctx.nodes.calls[since + 1]?.body).toEqual({
      id: row.id,
      quiesced: true,
    })
    expect(row).toMatchObject({
      purpose: 'snapshot',
      status: 'ready',
      local_id: `local-${row.id}`,
      object_key: `fleet/test/${key}/${row.id}.tar.gz`,
      attempts: 1,
    })
    expect(await ctx.kinds(key)).toEqual([
      'placement.created',
      'placement.placed',
      'snapshot.local',
      'snapshot.uploaded',
    ])
    expect(ctx.stored(key)).toEqual([`fleet/test/${key}/${row.id}.tar.gz`])
    expect(await ctx.runtime.goneSnapshots([taken.snapshot])).toEqual(new Set())

    // Deleted while the store is down: the node's copy goes now, the store's later.
    ctx.store.failing = true
    const deleting = ctx.nodes.calls.length
    await ctx.runtime.deleteSnapshot(taken.snapshot)
    expect(await ctx.callsSince(deleting)).toEqual([
      `backup-0 DELETE /v1/workloads/${key}/snapshots/local-${row.id} e- t${TEARDOWN_MS}`,
    ])
    expect((await ctx.archives(key))[0]).toMatchObject({
      status: 'deleted',
      object_key: `fleet/test/${key}/${row.id}.tar.gz`,
    })
    expect((await ctx.archives(key))[0]?.local_deleted_at).not.toBeNull()
    expect(ctx.logs.map((l) => l.message)).toContain('snapshot copies left for upkeep')
    expect(await ctx.runtime.goneSnapshots([taken.snapshot])).toEqual(new Set([taken.snapshot]))
    await ctx.runtime.dropDeletedCopies()
    expect(ctx.stored(key)).toHaveLength(1)
    ctx.store.failing = false
    await ctx.runtime.dropDeletedCopies()
    expect(ctx.stored(key)).toEqual([])
    expect((await ctx.archives(key))[0]).toMatchObject({
      status: 'deleted',
      object_key: null,
    })
    expect(await ctx.callsSince(deleting)).toHaveLength(1)
  })
  test('8. a planned move: stop, export, a new home restored, then the old copy fenced and deleted', async () => {
    const { key, handle, from, to } = await ctx.moving('planned')
    const fromName = await ctx.nameOf(from.id)
    const toName = await ctx.nameOf(to.id)
    const since = ctx.nodes.calls.length
    const after = await ctx.lastEvent()
    const { sink, seen } = ctx.recorder()
    const moved = await ctx.runtime.relocate(handle, { regionKey: 'planned' }, ctx.SPEC, sink)
    expect(await ctx.callsSince(since)).toEqual([
      `${fromName} GET /v1/workloads/${key} e- t-`,
      `${fromName} POST /v1/workloads/${key}/stop e1 t${TEARDOWN_MS}`,
      `${fromName} POST /v1/workloads/${key}/export e1 t${TRANSFER_MS}`,
      `${toName} PUT /v1/workloads/${key} e2 t1200000`,
      `${toName} POST /v1/workloads/${key}/restore e2 t${TRANSFER_MS}`,
      `${fromName} POST /v1/workloads/${key}/fence e- t${TEARDOWN_MS}`,
      `${fromName} DELETE /v1/workloads/${key}?data=delete e2 t${TEARDOWN_MS}`,
    ])
    const calls = ctx.nodes.calls.slice(since)
    const [archive] = await ctx.archives(key)
    if (archive === undefined) throw new Error('no archive')
    const objectKey = `fleet/test/${key}/${archive.id}.tar.gz`
    expect(calls[2]?.body).toEqual({
      quiesced: false,
      exclude: ['libraries'],
      url: expect.stringContaining(encodeURIComponent(objectKey)),
      headers: {},
    })
    expect(calls[4]?.body).toEqual({
      url: expect.stringContaining(encodeURIComponent(objectKey)),
      sha256: 'a'.repeat(64),
    })
    expect(calls[5]?.body).toEqual({ currentEpoch: 2 })
    expect(calls[6]?.headers).toEqual({ [ctx.CONFIRM]: key })
    expect(seen).toEqual(['storage', 'compute', 'handle e2'])
    expect(decodeHandle(moved)).toMatchObject({
      node: to.id,
      nodeName: toName,
      epoch: 2,
    })
    expect(await ctx.placement(key)).toMatchObject({
      node_id: to.id,
      epoch: '2',
      state: 'placed',
      desired_power: 'running',
      completing: null,
      restore_from: null,
      move_requested_at: null,
    })
    expect(await ctx.running(from.id)).toBe(0)
    expect(await ctx.running(to.id)).toBe(1024)
    expect(await ctx.history(key)).toEqual([
      { epoch: '1', node_id: from.id, reason: 'placed', end_reason: 'moved' },
      { epoch: '2', node_id: to.id, reason: 'move', end_reason: null },
    ])
    expect(await ctx.kinds(key, after)).toEqual([
      'move.exported',
      'placement.rehomed',
      'placement.placed',
      'placement.restored',
      'copy.fenced',
    ])
    expect(archive).toMatchObject({
      purpose: 'move',
      status: 'deleted',
      object_key: null,
      attempts: 1,
    })
    expect(ctx.stored(key)).toEqual([])
    expect(ctx.nodes.copies.has(`${from.id}/${key}`)).toBe(false)
    expect(ctx.nodes.copies.get(`${to.id}/${key}`)).toMatchObject({
      epoch: 2,
      state: 'created',
    })
  })
  test('9. a move with no room once its data is copied: the copy goes, and it runs where it was', async () => {
    const { key, handle, from, to } = await ctx.moving('no-room-after')
    const fromName = await ctx.nameOf(from.id)
    // The node it was to move to stops taking sandboxes while the data is copied.
    ctx.nodes.once(/^POST .*\/export$/, async () => {
      await drain(ctx.db, to.id, 'test')
    })
    const since = ctx.nodes.calls.length
    const after = await ctx.lastEvent()
    const moved = ctx.runtime.relocate(handle, { regionKey: 'no-room-after' }, ctx.SPEC, ctx.quiet)
    await expect(moved).rejects.toBeInstanceOf(RuntimeFull)
    // Every node is considered, in name order: the one it was asked to go to is draining now.
    const toName = await ctx.nameOf(to.id)
    const others = await rows<{ name: string }>(ctx.db, sql`SELECT name FROM fleet_nodes ORDER BY name`)
    const reasons = others.map(({ name }) =>
      name === fromName
        ? `${name}: excluded`
        : name === toName
          ? `${name}: lifecycle draining`
          : `${name}: not the node asked for`,
    )
    await expect(moved).rejects.toThrow(`The fleet runtime has no room: ${reasons.join('; ')}`)
    expect(await ctx.callsSince(since)).toEqual([
      `${fromName} GET /v1/workloads/${key} e- t-`,
      `${fromName} POST /v1/workloads/${key}/stop e1 t${TEARDOWN_MS}`,
      `${fromName} POST /v1/workloads/${key}/export e1 t${TRANSFER_MS}`,
      `${fromName} POST /v1/workloads/${key}/start e1 t60000`,
    ])
    expect(await ctx.placement(key)).toMatchObject({
      node_id: from.id,
      epoch: '1',
      state: 'placed',
      desired_power: 'running',
      completing: null,
    })
    expect((await ctx.placement(key))?.move_requested_at).not.toBeNull()
    expect(await ctx.running(from.id)).toBe(1024)
    expect(await ctx.running(to.id)).toBe(0)
    expect(await ctx.history(key)).toEqual([
      { epoch: '1', node_id: from.id, reason: 'placed', end_reason: null },
    ])
    expect(await ctx.kinds(key, after)).toEqual(['move.exported'])
    expect(await ctx.archives(key)).toMatchObject([{ purpose: 'move', status: 'deleted', object_key: null }])
    expect(ctx.stored(key)).toEqual([])
    expect(ctx.nodes.copies.get(`${from.id}/${key}`)).toMatchObject({
      epoch: 1,
      state: 'running',
    })
  })
  test('10. a move whose old copy its node refuses to fence is finished, and the copy left for upkeep', async () => {
    const refused = await ctx.moving('fence-refused')
    ctx.nodes.once(/^POST .*\/fence$/, async () => {
      throw new NodeRefused(StatusCodes.CONFLICT, 'busy', 'not now', null)
    })
    const fromName = await ctx.nameOf(refused.from.id)
    const since = ctx.nodes.calls.length
    const moved = await ctx.runtime.relocate(
      refused.handle,
      { regionKey: 'fence-refused' },
      ctx.SPEC,
      ctx.quiet,
    )
    expect(decodeHandle(moved)).toMatchObject({
      node: refused.to.id,
      epoch: 2,
    })
    expect((await ctx.callsSince(since)).filter((c) => c.startsWith(fromName))).toEqual([
      `${fromName} GET /v1/workloads/${refused.key} e- t-`,
      `${fromName} POST /v1/workloads/${refused.key}/stop e1 t${TEARDOWN_MS}`,
      `${fromName} POST /v1/workloads/${refused.key}/export e1 t${TRANSFER_MS}`,
      `${fromName} POST /v1/workloads/${refused.key}/fence e- t${TEARDOWN_MS}`,
    ])
    expect(ctx.logs.at(-1)).toEqual({
      message: 'move left its source copy for upkeep',
      fields: {
        node: refused.from.id,
        workload: refused.key,
        error: 'not now',
      },
    })
    expect(await ctx.eventData('copy.fenced', refused.key)).toEqual([])

    // Unreachable, the same; anything else is not the node's answer, and fails the move.
    const unreachable = await ctx.moving('fence-unreachable')
    ctx.nodes.once(/^POST .*\/fence$/, async () => {
      throw new NodeUnreachable('timeout', 'no answer')
    })
    await ctx.runtime.relocate(unreachable.handle, { regionKey: 'fence-unreachable' }, ctx.SPEC, ctx.quiet)
    expect(ctx.logs.at(-1)?.message).toBe('move left its source copy for upkeep')
    const broken = await ctx.moving('fence-broken')
    ctx.nodes.once(/^POST .*\/fence$/, async () => {
      throw new Error('a bug')
    })
    await expect(
      ctx.runtime.relocate(broken.handle, { regionKey: 'fence-broken' }, ctx.SPEC, ctx.quiet),
    ).rejects.toThrow('a bug')
    expect(await ctx.placement(broken.key)).toMatchObject({
      node_id: broken.to.id,
      epoch: '2',
      state: 'placed',
    })
  })
  test('11. a restore in place: a new epoch, the copy stopped, filled from its local snapshot, started', async () => {
    const [n] = (await ctx.region('restore', 1)) as [EnrolledNode]
    const key = ctx.server()
    const handle = await ctx.runtime.ensureProvisioned(key, { regionKey: 'restore' }, ctx.SPEC, ctx.quiet)
    const snap = await ctx.snapshotOn(n.id, key, null)
    const since = ctx.nodes.calls.length
    const after = await ctx.lastEvent()
    const { sink, seen } = ctx.recorder()
    const restored = await ctx.runtime.restore(
      handle,
      { kind: 'snapshot', snapshot: snap.handle },
      ctx.SPEC,
      sink,
    )
    expect(await ctx.callsSince(since)).toEqual([
      `restore-0 GET /v1/workloads/${key} e- t-`,
      `restore-0 POST /v1/workloads/${key}/stop e2 t${TEARDOWN_MS}`,
      `restore-0 PUT /v1/workloads/${key} e2 t1200000`,
      `restore-0 POST /v1/workloads/${key}/restore e2 t${TRANSFER_MS}`,
      `restore-0 POST /v1/workloads/${key}/start e2 t60000`,
    ])
    expect(ctx.nodes.calls[since + 3]?.body).toEqual({ snapshot: snap.id })
    expect(seen).toEqual(['storage', 'compute', 'handle e2'])
    expect(decodeHandle(restored)).toMatchObject({ node: n.id, epoch: 2 })
    expect(await ctx.placement(key)).toMatchObject({
      epoch: '2',
      state: 'placed',
      desired_power: 'running',
    })
    expect(await ctx.history(key)).toEqual([
      { epoch: '1', node_id: n.id, reason: 'placed', end_reason: 'restored' },
      { epoch: '2', node_id: n.id, reason: 'restore', end_reason: null },
    ])
    expect(await ctx.kinds(key, after)).toEqual([
      'placement.rehomed',
      'placement.placed',
      'placement.restored',
    ])
    // The old handle names a placement that is gone.
    await expect(ctx.runtime.stop(handle)).rejects.toThrow('This handle is for epoch 1; the server is at 2')
  })
  test('12. a release deletes its copy and keeps its backups; a destroy deletes everything', async () => {
    const [n] = (await ctx.region('letting-go', 1)) as [EnrolledNode]
    const key = ctx.server()
    const handle = await ctx.runtime.ensureProvisioned(key, { regionKey: 'letting-go' }, ctx.SPEC, ctx.quiet)
    const objectKey = `fleet/test/${key}/kept`
    const snap = await ctx.snapshotOn(n.id, key, objectKey)
    await ctx.pulse(n)
    const since = ctx.nodes.calls.length
    const after = await ctx.lastEvent()
    const released = await ctx.runtime.release(handle)
    expect(await ctx.callsSince(since)).toEqual([
      `letting-go-0 DELETE /v1/workloads/${key}?data=delete e1 t${TEARDOWN_MS}`,
    ])
    expect(ctx.nodes.calls[since]?.headers).toEqual({ [ctx.CONFIRM]: key })
    expect(decodeHandle(released)).toMatchObject({
      node: null,
      nodeName: null,
      epoch: 1,
      edgeHost: null,
    })
    expect(await ctx.placement(key)).toMatchObject({
      node_id: null,
      epoch: '1',
      state: 'released',
      completing: null,
    })
    expect(await ctx.history(key)).toEqual([
      { epoch: '1', node_id: n.id, reason: 'placed', end_reason: 'released' },
    ])
    expect((await ctx.archives(key))[0]).toMatchObject({
      id: snap.id,
      status: 'ready',
      object_key: objectKey,
    })
    expect((await ctx.archives(key))[0]?.local_deleted_at).not.toBeNull()
    expect(ctx.stored(key)).toEqual([objectKey])

    // Its node still reports the copy it held until its next beat: destroy asks it too.
    const destroying = ctx.nodes.calls.length
    await ctx.runtime.destroy(released)
    expect(await ctx.callsSince(destroying)).toEqual([
      `letting-go-0 DELETE /v1/workloads/${key}?data=delete e1 t${TEARDOWN_MS}`,
    ])
    expect(await ctx.placement(key)).toBeUndefined()
    expect(await ctx.history(key)).toEqual([
      { epoch: '1', node_id: n.id, reason: 'placed', end_reason: 'released' },
    ])
    expect(await ctx.archives(key)).toMatchObject([{ status: 'deleted', object_key: objectKey }])
    expect(ctx.stored(key)).toEqual([])
    expect(await ctx.kinds(key, after)).toEqual(['placement.released', 'placement.destroyed'])
    expect((await ctx.eventData('placement.destroyed', key)).map((e) => e.data)).toEqual([{ nodes: [n.id] }])
  })
  test('13. a suspended copy whose resume does not come back goes on as a boot', async () => {
    const [n] = (await ctx.region('resume-hung', 1)) as [EnrolledNode]
    const key = ctx.server()
    await ctx.runtime.ensureProvisioned(key, { regionKey: 'resume-hung' }, ctx.SPEC, ctx.quiet)
    Object.assign(ctx.nodes.copies.get(`${n.id}/${key}`) ?? {}, {
      state: 'suspended',
    })
    await ctx.pulse(n)
    ctx.nodes.once(/^POST .*\/start$/, async (answer) => ({
      ...(answer() as object),
      resumed: false,
    }))
    const { sink, seen } = ctx.recorder()
    await ctx.runtime.ensureProvisioned(key, { regionKey: 'resume-hung' }, ctx.SPEC, sink)
    expect(seen).toEqual(['allocating', 'handle e1', 'resuming', 'booting'])
  })
}
