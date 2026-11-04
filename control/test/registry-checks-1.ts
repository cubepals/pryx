// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Registry regression assertions registered against the original fixture. */

import 'reflect-metadata'
import { expect, test } from 'bun:test'
import { sql } from 'drizzle-orm'
import { pemSha256 } from '../src/ca.ts'
import { THRESHOLDS } from '../src/health.ts'
import { createToken, enroll, nodeSummaries, retire } from '../src/registry.ts'
import { rows } from '../src/sql.ts'
import type { WorkloadReport } from '../src/wire.ts'
import { beat, enrollNode, facts, nodeKey, registryOptions, report } from './nodes.ts'
import type { registryContext } from './registry-context.ts'

export function registerRegistry1(ctx: typeof registryContext) {
  test('a token enrolls one node, once, and expires', async () => {
    const { token } = await createToken(ctx.db, {
      regionKey: 'eu',
      ttlSeconds: 60,
      createdBy: 'test',
    })
    const key = await nodeKey()
    await enroll(ctx.db, ctx.ca, registryOptions(), {
      token,
      csrPem: key.csrPem,
      facts: facts('t1'),
    })
    // Another node, with a key of its own.
    await expect(
      enroll(ctx.db, ctx.ca, registryOptions(), {
        token,
        csrPem: (await nodeKey()).csrPem,
        facts: facts('t2'),
      }),
    ).rejects.toThrow('not valid')
    const stale = await createToken(ctx.db, {
      regionKey: 'eu',
      ttlSeconds: 60,
      createdBy: 'test',
    })
    await ctx.db.execute(
      sql`UPDATE fleet_node_tokens SET expires_at = now() - interval '1 second' WHERE id = ${stale.id}`,
    )
    await expect(
      enroll(ctx.db, ctx.ca, registryOptions(), {
        token: stale.token,
        csrPem: key.csrPem,
        facts: facts('t3'),
      }),
    ).rejects.toThrow('not valid')
  })
  test('a node whose answer was lost enrolls again with the same key, and nobody else can', async () => {
    const spent = await createToken(ctx.db, {
      regionKey: 'eu',
      ttlSeconds: 600,
      createdBy: 'test',
    })
    const [csr, again] = await ctx.requestsForOneKey()
    expect(again).not.toBe(csr)
    const first = await enroll(ctx.db, ctx.ca, registryOptions(), {
      token: spent.token,
      csrPem: csr,
      facts: facts('repeat'),
    })
    // The answer never arrived; the node asks again with a request signed anew for its key.
    const second = await enroll(ctx.db, ctx.ca, registryOptions(), {
      token: spent.token,
      csrPem: again,
      facts: facts('repeat'),
    })
    expect(second.nodeId).toBe(first.nodeId)
    expect(await rows(ctx.db, sql`SELECT id FROM fleet_nodes WHERE name = 'repeat'`)).toHaveLength(1)
    // The certificates it has now are the pinned ones; those it never received are not.
    const node = { id: second.nodeId, sha: pemSha256(second.clientCertPem) }
    await expect(
      ctx.send({ id: first.nodeId, sha: pemSha256(first.clientCertPem) }, beat(node, 's', 1)),
    ).rejects.toMatchObject({ code: 'certificate_not_current' })
    expect((await ctx.send(node, beat(node, 's', 2))).lifecycle).toBe('active')
    expect(
      (await ctx.events('node.enrollment_repeated')).filter((e) => e.node_id === first.nodeId),
    ).toHaveLength(1)
    // Another key gets what any token that isn't valid gets.
    const other = await nodeKey()
    await expect(
      enroll(ctx.db, ctx.ca, registryOptions(), {
        token: spent.token,
        csrPem: other.csrPem,
        facts: facts('thief'),
      }),
    ).rejects.toMatchObject({ code: 'invalid_token' })
    // So does the same key once the token's own time is up.
    await ctx.db.execute(
      sql`UPDATE fleet_node_tokens SET expires_at = now() - interval '1 second' WHERE id = ${spent.id}`,
    )
    await expect(
      enroll(ctx.db, ctx.ca, registryOptions(), {
        token: spent.token,
        csrPem: again,
        facts: facts('repeat'),
      }),
    ).rejects.toMatchObject({ code: 'invalid_token' })
  })
  test('a re-enrollment whose answer was lost is answered again; a retired node never is', async () => {
    const a = await enrollNode(ctx.db, ctx.ca, 'repeat-reenroll')
    const { token } = await createToken(ctx.db, {
      regionKey: '',
      ttlSeconds: 600,
      createdBy: 'test',
      node: a.id,
    })
    const [csr, again] = await ctx.requestsForOneKey()
    await enroll(ctx.db, ctx.ca, registryOptions(), {
      token,
      csrPem: csr,
      facts: facts('repeat-reenroll'),
    })
    const second = await enroll(ctx.db, ctx.ca, registryOptions(), {
      token,
      csrPem: again,
      facts: facts('repeat-reenroll'),
    })
    expect(second.nodeId).toBe(a.id)
    const renewed = { id: a.id, sha: pemSha256(second.clientCertPem) }
    expect((await ctx.send(renewed, beat(renewed, 's', 1))).lifecycle).toBe('active')
    await retire(ctx.db, a.id, 'test')
    await expect(
      enroll(ctx.db, ctx.ca, registryOptions(), {
        token,
        csrPem: again,
        facts: facts('repeat-reenroll'),
      }),
    ).rejects.toMatchObject({ code: 'invalid_token' })
  })
  test('only the hash of a token is kept', async () => {
    const { id, token } = await createToken(ctx.db, {
      regionKey: 'eu',
      ttlSeconds: 60,
      createdBy: 'test',
    })
    const [stored] = await rows<Record<string, unknown>>(
      ctx.db,
      sql`SELECT * FROM fleet_node_tokens WHERE id = ${id}`,
    )
    expect(JSON.stringify(stored)).not.toContain(token)
  })
  test("a heartbeat needs the node's current certificate", async () => {
    const a = await enrollNode(ctx.db, ctx.ca, 'pin')
    await expect(ctx.send({ ...a, sha: 'f'.repeat(64) }, beat(a, 's', 1))).rejects.toMatchObject({
      code: 'certificate_not_current',
    })
    const answer = await ctx.send(a, beat(a, 's', 1))
    expect(answer.lifecycle).toBe('active')
    expect(answer.leaseSeconds).toBe(120)
  })
  test('fences a copy whose server moved on, until the node reports it fenced', async () => {
    const a = await enrollNode(ctx.db, ctx.ca, 'fence-a')
    const b = await enrollNode(ctx.db, ctx.ca, 'fence-b')
    await ctx.place('w-moved', b.id, 2)
    const first = await ctx.send(a, beat(a, 's', 1, [report('w-moved', 1)]))
    expect(first.fences).toEqual([{ workload: 'w-moved', currentEpoch: 2 }])
    const after = await ctx.send(
      a,
      beat(a, 's', 2, [report('w-moved', 1, { state: 'fenced', supersededBy: 2 })]),
    )
    expect(after.fences).toEqual([])
    // The current copy is never fenced.
    expect((await ctx.send(b, beat(b, 's', 1, [report('w-moved', 2)]))).fences).toEqual([])
  })
  test('fences a copy of no placement at all, and reports it once', async () => {
    // As after its server was destroyed while this node couldn't be reached.
    const a = await enrollNode(ctx.db, ctx.ca, 'orphan')
    const first = await ctx.send(a, beat(a, 's', 1, [report('w-orphan', 3)]))
    expect(first.fences).toEqual([{ workload: 'w-orphan', currentEpoch: 4 }])
    const fenced = await ctx.send(
      a,
      beat(a, 's', 2, [report('w-orphan', 3, { state: 'fenced', supersededBy: 4 })]),
    )
    expect(fenced.fences).toEqual([])
    expect((await ctx.events('copy.orphaned')).filter((e) => e.workload === 'w-orphan')).toHaveLength(1)
  })
  test('never fences a copy a restore in place is taking over', async () => {
    const a = await enrollNode(ctx.db, ctx.ca, 'inplace')
    await ctx.place('w-inplace', a.id, 3, 'placing')
    expect(
      (await ctx.send(a, beat(a, 's', 1, [report('w-inplace', 2, { state: 'stopped' })]))).fences,
    ).toEqual([])
  })
  test('adopts a newer epoch its own node reports (a restored database), and flags one from elsewhere', async () => {
    const a = await enrollNode(ctx.db, ctx.ca, 'adopt-a')
    const b = await enrollNode(ctx.db, ctx.ca, 'adopt-b')
    await ctx.place('w-adopt', a.id, 1)
    await ctx.send(a, beat(a, 's', 1, [report('w-adopt', 4)]))
    const [placement] = await rows<{ epoch: string }>(
      ctx.db,
      sql`SELECT epoch FROM fleet_placements WHERE workload = 'w-adopt'`,
    )
    expect(Number(placement?.epoch)).toBe(4)
    await ctx.send(b, beat(b, 's', 1, [report('w-adopt', 9)]))
    expect((await ctx.events('epoch.conflict')).some((e) => e.workload === 'w-adopt')).toBe(true)
  })
  test('a delayed beat changes nothing, and a node that reports nothing holds nothing', async () => {
    const a = await enrollNode(ctx.db, ctx.ca, 'late')
    await ctx.send(a, beat(a, 's', 5, [report('w-late', 1, { state: 'stopped' })]))
    await ctx.send(a, beat(a, 's', 4, [report('w-late', 1, { state: 'running' })]))
    const [observed] = await rows<{ state: string }>(
      ctx.db,
      sql`SELECT state FROM fleet_observations WHERE workload = 'w-late'`,
    )
    expect(observed?.state).toBe('stopped')
    await ctx.send(a, beat(a, 's', 6, []))
    expect(await rows(ctx.db, sql`SELECT * FROM fleet_observations WHERE node_id = ${a.id}`)).toHaveLength(0)
  })
  test('what a node finds wrong with a workload is kept with its report, for operators to read', async () => {
    const a = await enrollNode(ctx.db, ctx.ca, 'issues')
    const refused = {
      code: 'insufficient_capacity',
      detail: 'not restarted after a failure: no room',
    }
    await ctx.send(a, beat(a, 's', 1, [report('w-issues', 1, { state: 'crashed', issues: [refused] })]))
    const read = async () =>
      (
        await rows<{ report: WorkloadReport }>(
          ctx.db,
          sql`SELECT report FROM fleet_observations WHERE workload = 'w-issues'`,
        )
      )[0]?.report.issues
    expect(await read()).toEqual([refused])
    // Mended, the node says nothing more, and the report says so too.
    await ctx.send(a, beat(a, 's', 2, [report('w-issues', 1, { state: 'running' })]))
    expect(await read()).toBeUndefined()
  })
  test('an unchanged report writes nothing', async () => {
    const a = await enrollNode(ctx.db, ctx.ca, 'quiet')
    const steady = report('w-quiet', 1, { changedAt: '2026-01-01T00:00:00Z' })
    await ctx.send(a, beat(a, 's', 1, [steady]))
    const [before] = await rows<{ observed_at: Date }>(
      ctx.db,
      sql`SELECT observed_at FROM fleet_observations WHERE workload = 'w-quiet'`,
    )
    await Bun.sleep(20)
    await ctx.send(a, beat(a, 's', 2, [steady]))
    const [after] = await rows<{ observed_at: Date }>(
      ctx.db,
      sql`SELECT observed_at FROM fleet_observations WHERE workload = 'w-quiet'`,
    )
    expect(after?.observed_at).toEqual(before?.observed_at as Date)
  })
  test('a daemon restart is a new session, not a clone', async () => {
    const a = await enrollNode(ctx.db, ctx.ca, 'restart')
    await ctx.send(a, beat(a, 'one', 1))
    await ctx.send(a, beat(a, 'one', 2))
    await ctx.send(a, beat(a, 'two', 1))
    // The old daemon's last beat, still in flight: refused, but no quarantine.
    await expect(ctx.send(a, beat(a, 'one', 3))).rejects.toMatchObject({
      code: 'session_replaced',
    })
    const summary = (await nodeSummaries(ctx.db, THRESHOLDS)).find((n) => n.id === a.id)
    expect(summary?.duplicate_seen_at).toBeNull()
    expect((await ctx.events('node.daemon_restarted')).some((e) => e.node_id === a.id)).toBe(true)
  })
  test('a second host with the same identity is caught and quarantined', async () => {
    const a = await enrollNode(ctx.db, ctx.ca, 'clone')
    for (let seq = 1; seq <= 5; seq++) await ctx.send(a, beat(a, 'original', seq))
    // A copy of the disk has been beating on its own for a while.
    await expect(ctx.send(a, beat(a, 'clone', 40))).rejects.toMatchObject({
      code: 'duplicate_identity',
    })
    const summary = (await nodeSummaries(ctx.db, THRESHOLDS)).find((n) => n.id === a.id)
    expect(summary?.duplicate_seen_at).not.toBeNull()
    expect((await ctx.events('node.duplicate_identity')).filter((e) => e.node_id === a.id)).toHaveLength(1)
    // The original keeps reporting, and gets nothing new placed on it until an operator decides.
    expect((await ctx.send(a, beat(a, 'original', 6))).lifecycle).toBe('active')
  })
}
