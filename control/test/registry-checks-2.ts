// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Registry regression assertions registered against the original fixture. */

import 'reflect-metadata'
import { expect, setSystemTime, test } from 'bun:test'
import { sql } from 'drizzle-orm'
import { pemSha256 } from '../src/ca.ts'
import { DAY_MS } from '../src/durations.ts'
import { THRESHOLDS } from '../src/health.ts'
import {
  confirmLost,
  createToken,
  drain,
  endpointSeen,
  enroll,
  holdsMemory,
  listeningSeconds,
  nodeSummaries,
  Refused,
  reinstate,
  renew,
  retire,
  sessionOf,
} from '../src/registry.ts'
import { rows } from '../src/sql.ts'
import { beat, type EnrolledNode, enrollNode, facts, nodeKey, registryOptions, report } from './nodes.ts'
import type { registryContext } from './registry-context.ts'

export function registerRegistry2(ctx: typeof registryContext) {
  test('a node that moves to a new address is reached there, and the change is recorded', async () => {
    const a = await enrollNode(ctx.db, ctx.ca, 'moving', {
      apiAddress: '10.0.0.5:7443',
    })
    await ctx.send(
      a,
      beat(a, 's', 1, [], {
        addresses: {
          api: '10.0.9.5:7443',
          edge: '10.0.9.5',
          control: '10.0.9.5',
        },
      }),
    )
    const summary = (await nodeSummaries(ctx.db, THRESHOLDS)).find((n) => n.id === a.id)
    expect([summary?.api_address, summary?.edge_host, summary?.control_host]).toEqual([
      '10.0.9.5:7443',
      '10.0.9.5',
      '10.0.9.5',
    ])
    expect((await ctx.events('node.addresses_changed')).some((e) => e.node_id === a.id)).toBe(true)
    // Something that isn't an address is ignored, not stored.
    await ctx.send(
      a,
      beat(a, 's', 2, [], {
        addresses: { api: 'http://x/?', edge: 'a b', control: '10.0.9.5' },
      }),
    )
    expect((await nodeSummaries(ctx.db, THRESHOLDS)).find((n) => n.id === a.id)?.api_address).toBe(
      '10.0.9.5:7443',
    )
  })
  test('renewal: the new certificate works alongside the old until a heartbeat brings it', async () => {
    const a = await enrollNode(ctx.db, ctx.ca, 'renew')
    await ctx.send(a, beat(a, 's', 1))
    // Due: the current certificate ends within the renewal window.
    await ctx.db.execute(
      sql`UPDATE fleet_nodes SET cert_expires_at = now() + interval '5 days' WHERE id = ${a.id}`,
    )
    expect((await ctx.send(a, beat(a, 's', 2))).renew).toBe(true)
    const key = await nodeKey()
    const renewed = await renew(ctx.db, ctx.ca, registryOptions(), a.id, a.sha, {
      nodeId: a.id,
      csrPem: key.csrPem,
    })
    const next = { id: a.id, sha: pemSha256(renewed.clientCertPem) }
    // The old one still works: a renewal the node never received doesn't lock it out.
    expect((await ctx.send(a, beat(a, 's', 3))).lifecycle).toBe('active')
    // The first heartbeat with the new one makes it the only one.
    expect((await ctx.send(next, beat(a, 's', 4))).renew).toBeUndefined()
    await expect(ctx.send(a, beat(a, 's', 5))).rejects.toMatchObject({
      code: 'certificate_not_current',
    })
    expect((await ctx.events('node.certificate_renewed')).some((e) => e.node_id === a.id)).toBe(true)
  })
  test('a certificate is due for renewal by the database’s clock, whatever this process’s says', async () => {
    const a = await enrollNode(ctx.db, ctx.ca, 'renew-clock')
    await ctx.send(a, beat(a, 's', 1))
    const left = (days: number) =>
      ctx.db.execute(
        sql`UPDATE fleet_nodes SET cert_expires_at = now() + make_interval(days => ${days}) WHERE id = ${a.id}`,
      )
    const now = Date.now()
    try {
      // Three weeks fast: 25 days left is not yet within the 10 days before the end.
      await left(25)
      setSystemTime(new Date(now + 21 * DAY_MS))
      expect((await ctx.send(a, beat(a, 's', 2))).renew).toBeUndefined()
      // Three weeks slow: 5 days left is.
      await left(5)
      setSystemTime(new Date(now - 21 * DAY_MS))
      expect((await ctx.send(a, beat(a, 's', 3))).renew).toBe(true)
    } finally {
      setSystemTime()
    }
  })
  test("renewal needs the node's current certificate, and a node in good standing", async () => {
    const a = await enrollNode(ctx.db, ctx.ca, 'renew-refused')
    const key = await nodeKey()
    await expect(
      renew(ctx.db, ctx.ca, registryOptions(), a.id, 'f'.repeat(64), {
        nodeId: a.id,
        csrPem: key.csrPem,
      }),
    ).rejects.toMatchObject({
      code: 'certificate_not_current',
    })
    await ctx.db.execute(sql`UPDATE fleet_nodes SET duplicate_seen_at = now() WHERE id = ${a.id}`)
    await expect(
      renew(ctx.db, ctx.ca, registryOptions(), a.id, a.sha, {
        nodeId: a.id,
        csrPem: key.csrPem,
      }),
    ).rejects.toMatchObject({
      code: 'quarantined',
    })
  })
  test('a node enrolls again under its own id, and its old certificate stops working', async () => {
    const a = await enrollNode(ctx.db, ctx.ca, 'reenroll')
    await ctx.place('w-reenroll', a.id, 5)
    await ctx.send(a, beat(a, 'before', 1, [report('w-reenroll', 5)]))
    const { token } = await createToken(ctx.db, {
      regionKey: '',
      ttlSeconds: 60,
      createdBy: 'test',
      node: a.id,
    })
    const key = await nodeKey()
    const again = await enroll(ctx.db, ctx.ca, registryOptions(), {
      token,
      csrPem: key.csrPem,
      facts: facts('reenroll'),
    })
    expect(again.nodeId).toBe(a.id)
    // The old certificate is refused; the new one beats, in a new session, keeping the placement.
    await expect(ctx.send(a, beat(a, 'before', 2))).rejects.toMatchObject({
      code: 'certificate_not_current',
    })
    const renewed = { id: a.id, sha: pemSha256(again.clientCertPem) }
    const answer = await ctx.send(renewed, beat(renewed, 'after', 1, [report('w-reenroll', 5)]))
    expect(answer.fences).toEqual([])
    const [placement] = await rows<{ node_id: string }>(
      ctx.db,
      sql`SELECT node_id FROM fleet_placements WHERE workload = 'w-reenroll'`,
    )
    expect(placement?.node_id).toBe(a.id)
    expect((await ctx.events('node.reenrolled')).map((e) => e.node_id)).toContain(a.id)
    // Once: another key can't use it again.
    await expect(
      enroll(ctx.db, ctx.ca, registryOptions(), {
        token,
        csrPem: (await nodeKey()).csrPem,
        facts: facts('reenroll'),
      }),
    ).rejects.toMatchObject({ code: 'invalid_token' })
  })
  test('loss is confirmed, never timed; a returning copy is fenced, gets no lease, and is recorded as a fork', async () => {
    const a = await enrollNode(ctx.db, ctx.ca, 'lost-a')
    const b = await enrollNode(ctx.db, ctx.ca, 'lost-b')
    await ctx.send(a, beat(a, 's', 1, [report('w-lost', 1)]))
    await ctx.place('w-lost', a.id, 1)
    // It is beating: an operator can't declare it lost without forcing.
    await expect(
      confirmLost(ctx.db, THRESHOLDS, a.id, {
        fencedBy: 'test',
        reason: 'test',
        by: 'test',
      }),
    ).rejects.toBeInstanceOf(Refused)
    await ctx.silence(a.id)
    const lost = await confirmLost(ctx.db, THRESHOLDS, a.id, {
      fencedBy: 'powered off (test)',
      reason: 'disk died',
      by: 'test',
    })
    expect(lost.displaced).toEqual(['w-lost'])
    // Rebuilt elsewhere from a snapshot: epoch 1 ended because its host was lost.
    await ctx.db.execute(
      sql`UPDATE fleet_placement_history SET ended_at = now(), end_reason = 'lost' WHERE workload = 'w-lost'`,
    )
    await ctx.place('w-lost', b.id, 2)
    // The lost node comes back, still holding epoch 1, in a new session that counts once it can.
    await ctx.send(a, beat(a, 's2', 1, [report('w-lost', 1)])).then(() => ctx.pastHold(a.id))
    const back = await ctx.send(a, beat(a, 's2', 2, [report('w-lost', 1)]))
    expect([back.lifecycle, back.leaseSeconds]).toEqual(['lost', 0])
    expect(back.fences).toEqual([{ workload: 'w-lost', currentEpoch: 2 }])
    expect((await ctx.events('fork.detected')).some((e) => e.workload === 'w-lost')).toBe(true)
    expect((await ctx.events('node.returned_while_lost')).some((e) => e.node_id === a.id)).toBe(true)
    // A lost node's certificates are not renewed.
    const key = await nodeKey()
    await expect(
      renew(ctx.db, ctx.ca, registryOptions(), a.id, a.sha, {
        nodeId: a.id,
        csrPem: key.csrPem,
      }),
    ).rejects.toMatchObject({
      code: 'lost',
    })
  })
  test('a node that comes back before its sandboxes were rebuilt can be reinstated', async () => {
    const a = await enrollNode(ctx.db, ctx.ca, 'reinstate')
    await ctx.place('w-back', a.id, 1)
    await ctx.silence(a.id)
    await confirmLost(ctx.db, THRESHOLDS, a.id, {
      fencedBy: 'test',
      reason: 'test',
      by: 'test',
    })
    expect((await reinstate(ctx.db, a.id, 'test')).restored).toEqual(['w-back'])
    const [placement] = await rows<{ state: string }>(
      ctx.db,
      sql`SELECT state FROM fleet_placements WHERE workload = 'w-back'`,
    )
    expect(placement?.state).toBe('placed')
  })
  test('retiring needs an empty node, and its certificates stop working', async () => {
    const a = await enrollNode(ctx.db, ctx.ca, 'retire')
    await ctx.place('w-retire', a.id, 1)
    await expect(retire(ctx.db, a.id, 'test')).rejects.toMatchObject({
      code: 'not_empty',
    })
    await ctx.db.execute(
      sql`UPDATE fleet_placements SET state = 'released', node_id = NULL WHERE workload = 'w-retire'`,
    )
    await retire(ctx.db, a.id, 'test')
    await expect(ctx.send(a, beat(a, 's', 1))).rejects.toMatchObject({
      code: 'retired',
    })
  })
  test('draining is an operator’s, and recorded with who asked', async () => {
    const a = await enrollNode(ctx.db, ctx.ca, 'drain')
    await drain(ctx.db, a.id, 'operator:ada')
    await expect(drain(ctx.db, a.id, 'operator:ada')).rejects.toMatchObject({
      code: 'wrong_lifecycle',
    })
    expect((await ctx.events('node.draining')).find((e) => e.node_id === a.id)?.data.by).toBe('operator:ada')
  })
  test('the ledger counts what placements promised on each node', async () => {
    const a = await enrollNode(ctx.db, ctx.ca, 'ledger')
    await ctx.place('w-l1', a.id, 1)
    await ctx.place('w-l2', a.id, 1, 'placing')
    await ctx.place('w-l3', a.id, 1, 'displaced')
    const summary = (await nodeSummaries(ctx.db, THRESHOLDS)).find((n) => n.id === a.id)
    expect(summary?.allocated).toEqual({
      memoryMb: 2048,
      cpuMillis: 500,
      diskGb: 6,
      workloads: 2,
    })
  })
  test('only sandboxes that run, or are being started or made, hold memory', async () => {
    const a = await enrollNode(ctx.db, ctx.ca, 'admission')
    /** The last power command, `ago` seconds back. */
    const power = (workload: string, desired: 'running' | 'stopped', ago: number) =>
      ctx.db.execute(sql`
        UPDATE fleet_placements SET desired_power = ${desired}, power_changed_at = now() - make_interval(secs => ${ago})
        WHERE workload = ${workload}`)
    /** What the node reported of the copy, changed `ago` seconds back. */
    const observe = (workload: string, state: string, ago: number) =>
      ctx.db.execute(sql`
        INSERT INTO fleet_observations (node_id, workload, epoch, state, report, state_changed_at)
        VALUES (${a.id}, ${workload}, 1, ${state}, '{}'::jsonb, now() - make_interval(secs => ${ago}))`)
    // Asleep: told to stop, and stopped.
    await ctx.place('w-h1', a.id, 1)
    await power('w-h1', 'stopped', 600)
    await observe('w-h1', 'stopped', 590)
    // Being made.
    await ctx.place('w-h2', a.id, 1, 'placing')
    // Told to start; the node hasn't reported since.
    await ctx.place('w-h3', a.id, 1)
    await power('w-h3', 'running', 60)
    // Running.
    await ctx.place('w-h4', a.id, 1)
    await power('w-h4', 'running', 600)
    await observe('w-h4', 'running', 590)
    // Crashed after it was started: it holds nothing until it is started again.
    await ctx.place('w-h5', a.id, 1)
    await power('w-h5', 'running', 600)
    await observe('w-h5', 'crashed', 10)
    // Started just now; the node last reported it stopped, before the start.
    await ctx.place('w-h6', a.id, 1)
    await power('w-h6', 'running', 5)
    await observe('w-h6', 'stopped', 600)
    // Told to stop, but the node still runs it.
    await ctx.place('w-h7', a.id, 1)
    await power('w-h7', 'stopped', 60)
    await observe('w-h7', 'running', 600)
    // Made and told to start; the node reports it made, not yet started.
    await ctx.place('w-h8', a.id, 1)
    await power('w-h8', 'running', 60)
    await observe('w-h8', 'created', 10)
    // Its host was lost: it holds disk on the node, nothing else.
    await ctx.place('w-h9', a.id, 1, 'displaced')
    const summary = (await nodeSummaries(ctx.db, THRESHOLDS)).find((n) => n.id === a.id)
    expect(summary?.running).toEqual({
      memoryMb: 6 * 1024,
      cpuMillis: 6 * 250,
      workloads: 6,
    })
    expect(summary?.allocated).toEqual({
      memoryMb: 8 * 1024,
      cpuMillis: 8 * 250,
      diskGb: 9 * 2,
      workloads: 8,
    })
    const holding = await Promise.all(
      ['w-h1', 'w-h2', 'w-h5', 'w-h6', 'w-h7', 'w-h9'].map((w) => holdsMemory(ctx.db, w)),
    )
    expect(holding).toEqual([false, true, false, true, true, false])
  })
  test("silence while no endpoint listened isn't held against a node", async () => {
    const a: EnrolledNode = await enrollNode(ctx.db, ctx.ca, 'outage')
    await ctx.send(a, beat(a, 's', 1))
    await ctx.silence(a.id)
    expect((await nodeSummaries(ctx.db, THRESHOLDS)).find((n) => n.id === a.id)?.derivedHealth).toBe(
      'unavailable',
    )
    // Every endpoint went away; one just came back.
    await ctx.db.execute(sql`DELETE FROM fleet_endpoints`)
    await endpointSeen(ctx.db, 'fresh', new Date())
    expect(await listeningSeconds(ctx.db)).toBeLessThan(5)
    expect((await nodeSummaries(ctx.db, THRESHOLDS)).find((n) => n.id === a.id)?.derivedHealth).toBe(
      'suspect',
    )
  })
  test('sessions: a stale sequence, a restart and a clone are told apart', () => {
    const node = {
      session_id: 'a',
      heartbeat_seq: '10',
      previous_session_id: null,
      session_age: 600,
      age: 2,
    }
    expect(sessionOf(node, { sessionId: 'a', seq: 11 })).toBe('current')
    expect(sessionOf(node, { sessionId: 'a', seq: 10 })).toBe('stale')
    expect(sessionOf(node, { sessionId: 'b', seq: 1 })).toBe('switched')
    expect(sessionOf(node, { sessionId: 'b', seq: 50 })).toBe('duplicate')
    expect(
      sessionOf({ ...node, previous_session_id: 'z', session_age: 5 }, { sessionId: 'z', seq: 99 }),
    ).toBe('replaced')
    expect(
      sessionOf({ ...node, previous_session_id: 'z', session_age: 60 }, { sessionId: 'z', seq: 99 }),
    ).toBe('duplicate')
  })
}
