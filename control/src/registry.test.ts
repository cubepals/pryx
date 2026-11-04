// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

import 'reflect-metadata'
import { afterAll, beforeAll, describe, expect, setSystemTime, test } from 'bun:test'
import { webcrypto } from 'node:crypto'
import { DAY_MS } from '@blockly/contracts/durations'
import type { Db } from '@blockly/db'
import * as x509 from '@peculiar/x509'
import { sql } from 'drizzle-orm'
import {
  beat,
  type EnrolledNode,
  enrollNode,
  facts,
  freshDatabase,
  hasDatabase,
  nodeKey,
  registryOptions,
  report,
  testCa,
} from '../../testing/fleet.ts'
import { type FleetCa, pemSha256 } from './ca.ts'
import { THRESHOLDS } from './health.ts'
import { caSha256, joinToken } from './join-token.ts'
import {
  answered,
  confirmLost,
  createToken,
  drain,
  endpointSeen,
  enroll,
  heartbeat,
  holdsMemory,
  listeningSeconds,
  nodeSummaries,
  Refused,
  reinstate,
  renew,
  retire,
  sessionOf,
} from './registry.ts'
import { rows } from './sql.ts'
import type { HeartbeatRequest, WorkloadReport, WorkloadView } from './wire.ts'

let db: Db
let ca: FleetCa
let drop: () => Promise<void>

const send = (n: { id: string; sha: string }, request: HeartbeatRequest) =>
  heartbeat(db, registryOptions(), n.id, n.sha, request)

async function place(workload: string, nodeId: string, epoch: number, state = 'placed'): Promise<void> {
  await db.execute(sql`
    INSERT INTO fleet_placements (workload, node_id, epoch, state, region_key, memory_mb, cpu_millis, disk_gb)
    VALUES (${workload}, ${nodeId}, ${epoch}, ${state}, 'eu', 1024, 250, 2)
    ON CONFLICT (workload) DO UPDATE SET node_id = ${nodeId}, epoch = ${epoch}, state = ${state}`)
  await db.execute(sql`
    INSERT INTO fleet_placement_history (workload, epoch, node_id, reason) VALUES (${workload}, ${epoch}, ${nodeId}, 'test')
    ON CONFLICT DO NOTHING`)
}

const events = (kind: string) =>
  rows<{ workload: string; node_id: string; data: Record<string, unknown> }>(
    db,
    sql`SELECT * FROM fleet_events WHERE kind = ${kind}`,
  )

const silence = (id: string, minutes = 10) =>
  db.execute(
    sql`UPDATE fleet_nodes SET last_heartbeat_at = now() - make_interval(mins => ${minutes}) WHERE id = ${id}`,
  )

/** The session's switch, as long ago as the new session's hold lasts. */
const pastHold = (id: string) =>
  db.execute(sql`UPDATE fleet_nodes SET session_started_at = now() - interval '30 seconds' WHERE id = ${id}`)

const ALG = { name: 'ECDSA', namedCurve: 'P-256', hash: 'SHA-256' }

/** Two requests for one key, as a node that keeps its key between attempts signs them. */
async function requestsForOneKey(): Promise<[string, string]> {
  const keys = (await webcrypto.subtle.generateKey(ALG, true, ['sign', 'verify'])) as CryptoKeyPair
  const request = async () =>
    (
      await x509.Pkcs10CertificateRequestGenerator.create({ name: 'CN=node', keys, signingAlgorithm: ALG })
    ).toString('pem')
  return [await request(), await request()]
}

describe.skipIf(!hasDatabase)('the node registry, against Postgres', () => {
  beforeAll(async () => {
    ;({ db, drop } = await freshDatabase())
    ca = await testCa()
    // A node endpoint has been serving for a while, so silence counts against nodes.
    await db.execute(sql`INSERT INTO fleet_endpoints VALUES ('test', now() - interval '1 hour', now())`)
  })
  afterAll(() => drop?.())

  test('a token enrolls one node, once, and expires', async () => {
    const { token } = await createToken(db, { regionKey: 'eu', ttlSeconds: 60, createdBy: 'test' })
    const key = await nodeKey()
    await enroll(db, ca, registryOptions(), { token, csrPem: key.csrPem, facts: facts('t1') })
    // Another node, with a key of its own.
    await expect(
      enroll(db, ca, registryOptions(), { token, csrPem: (await nodeKey()).csrPem, facts: facts('t2') }),
    ).rejects.toThrow('not valid')
    const stale = await createToken(db, { regionKey: 'eu', ttlSeconds: 60, createdBy: 'test' })
    await db.execute(
      sql`UPDATE fleet_node_tokens SET expires_at = now() - interval '1 second' WHERE id = ${stale.id}`,
    )
    await expect(
      enroll(db, ca, registryOptions(), { token: stale.token, csrPem: key.csrPem, facts: facts('t3') }),
    ).rejects.toThrow('not valid')
  })

  test('a node whose answer was lost enrolls again with the same key, and nobody else can', async () => {
    const spent = await createToken(db, { regionKey: 'eu', ttlSeconds: 600, createdBy: 'test' })
    const [csr, again] = await requestsForOneKey()
    expect(again).not.toBe(csr)
    const first = await enroll(db, ca, registryOptions(), {
      token: spent.token,
      csrPem: csr,
      facts: facts('repeat'),
    })
    // The answer never arrived; the node asks again with a request signed anew for its key.
    const second = await enroll(db, ca, registryOptions(), {
      token: spent.token,
      csrPem: again,
      facts: facts('repeat'),
    })
    expect(second.nodeId).toBe(first.nodeId)
    expect(await rows(db, sql`SELECT id FROM fleet_nodes WHERE name = 'repeat'`)).toHaveLength(1)
    // The certificates it has now are the pinned ones; those it never received are not.
    const node = { id: second.nodeId, sha: pemSha256(second.clientCertPem) }
    await expect(
      send({ id: first.nodeId, sha: pemSha256(first.clientCertPem) }, beat(node, 's', 1)),
    ).rejects.toMatchObject({ code: 'certificate_not_current' })
    expect((await send(node, beat(node, 's', 2))).lifecycle).toBe('active')
    expect((await events('node.enrollment_repeated')).filter((e) => e.node_id === first.nodeId)).toHaveLength(
      1,
    )
    // Another key gets what any token that isn't valid gets.
    const other = await nodeKey()
    await expect(
      enroll(db, ca, registryOptions(), { token: spent.token, csrPem: other.csrPem, facts: facts('thief') }),
    ).rejects.toMatchObject({ code: 'invalid_token' })
    // So does the same key once the token's own time is up.
    await db.execute(
      sql`UPDATE fleet_node_tokens SET expires_at = now() - interval '1 second' WHERE id = ${spent.id}`,
    )
    await expect(
      enroll(db, ca, registryOptions(), { token: spent.token, csrPem: again, facts: facts('repeat') }),
    ).rejects.toMatchObject({ code: 'invalid_token' })
  })

  test('a re-enrollment whose answer was lost is answered again; a retired node never is', async () => {
    const a = await enrollNode(db, ca, 'repeat-reenroll')
    const { token } = await createToken(db, { regionKey: '', ttlSeconds: 600, createdBy: 'test', node: a.id })
    const [csr, again] = await requestsForOneKey()
    await enroll(db, ca, registryOptions(), { token, csrPem: csr, facts: facts('repeat-reenroll') })
    const second = await enroll(db, ca, registryOptions(), {
      token,
      csrPem: again,
      facts: facts('repeat-reenroll'),
    })
    expect(second.nodeId).toBe(a.id)
    const renewed = { id: a.id, sha: pemSha256(second.clientCertPem) }
    expect((await send(renewed, beat(renewed, 's', 1))).lifecycle).toBe('active')
    await retire(db, a.id, 'test')
    await expect(
      enroll(db, ca, registryOptions(), { token, csrPem: again, facts: facts('repeat-reenroll') }),
    ).rejects.toMatchObject({ code: 'invalid_token' })
  })

  test('only the hash of a token is kept', async () => {
    const { id, token } = await createToken(db, { regionKey: 'eu', ttlSeconds: 60, createdBy: 'test' })
    const [stored] = await rows<Record<string, unknown>>(
      db,
      sql`SELECT * FROM fleet_node_tokens WHERE id = ${id}`,
    )
    expect(JSON.stringify(stored)).not.toContain(token)
  })

  test("a heartbeat needs the node's current certificate", async () => {
    const a = await enrollNode(db, ca, 'pin')
    await expect(send({ ...a, sha: 'f'.repeat(64) }, beat(a, 's', 1))).rejects.toMatchObject({
      code: 'certificate_not_current',
    })
    const answer = await send(a, beat(a, 's', 1))
    expect(answer.lifecycle).toBe('active')
    expect(answer.leaseSeconds).toBe(120)
  })

  test('fences a copy whose server moved on, until the node reports it fenced', async () => {
    const a = await enrollNode(db, ca, 'fence-a')
    const b = await enrollNode(db, ca, 'fence-b')
    await place('w-moved', b.id, 2)
    const first = await send(a, beat(a, 's', 1, [report('w-moved', 1)]))
    expect(first.fences).toEqual([{ workload: 'w-moved', currentEpoch: 2 }])
    const after = await send(a, beat(a, 's', 2, [report('w-moved', 1, { state: 'fenced', supersededBy: 2 })]))
    expect(after.fences).toEqual([])
    // The current copy is never fenced.
    expect((await send(b, beat(b, 's', 1, [report('w-moved', 2)]))).fences).toEqual([])
  })

  test('fences a copy of no placement at all, and reports it once', async () => {
    // As after its server was destroyed while this node couldn't be reached.
    const a = await enrollNode(db, ca, 'orphan')
    const first = await send(a, beat(a, 's', 1, [report('w-orphan', 3)]))
    expect(first.fences).toEqual([{ workload: 'w-orphan', currentEpoch: 4 }])
    const fenced = await send(
      a,
      beat(a, 's', 2, [report('w-orphan', 3, { state: 'fenced', supersededBy: 4 })]),
    )
    expect(fenced.fences).toEqual([])
    expect((await events('copy.orphaned')).filter((e) => e.workload === 'w-orphan')).toHaveLength(1)
  })

  test('never fences a copy a restore in place is taking over', async () => {
    const a = await enrollNode(db, ca, 'inplace')
    await place('w-inplace', a.id, 3, 'placing')
    expect((await send(a, beat(a, 's', 1, [report('w-inplace', 2, { state: 'stopped' })]))).fences).toEqual(
      [],
    )
  })

  test('adopts a newer epoch its own node reports (a restored database), and flags one from elsewhere', async () => {
    const a = await enrollNode(db, ca, 'adopt-a')
    const b = await enrollNode(db, ca, 'adopt-b')
    await place('w-adopt', a.id, 1)
    await send(a, beat(a, 's', 1, [report('w-adopt', 4)]))
    const [placement] = await rows<{ epoch: string }>(
      db,
      sql`SELECT epoch FROM fleet_placements WHERE workload = 'w-adopt'`,
    )
    expect(Number(placement?.epoch)).toBe(4)
    await send(b, beat(b, 's', 1, [report('w-adopt', 9)]))
    expect((await events('epoch.conflict')).some((e) => e.workload === 'w-adopt')).toBe(true)
  })

  test('a delayed beat changes nothing, and a node that reports nothing holds nothing', async () => {
    const a = await enrollNode(db, ca, 'late')
    await send(a, beat(a, 's', 5, [report('w-late', 1, { state: 'stopped' })]))
    await send(a, beat(a, 's', 4, [report('w-late', 1, { state: 'running' })]))
    const [observed] = await rows<{ state: string }>(
      db,
      sql`SELECT state FROM fleet_observations WHERE workload = 'w-late'`,
    )
    expect(observed?.state).toBe('stopped')
    await send(a, beat(a, 's', 6, []))
    expect(await rows(db, sql`SELECT * FROM fleet_observations WHERE node_id = ${a.id}`)).toHaveLength(0)
  })

  test('what a node finds wrong with a workload is kept with its report, for operators to read', async () => {
    const a = await enrollNode(db, ca, 'issues')
    const refused = { code: 'insufficient_capacity', detail: 'not restarted after a failure: no room' }
    await send(a, beat(a, 's', 1, [report('w-issues', 1, { state: 'crashed', issues: [refused] })]))
    const read = async () =>
      (
        await rows<{ report: WorkloadReport }>(
          db,
          sql`SELECT report FROM fleet_observations WHERE workload = 'w-issues'`,
        )
      )[0]?.report.issues
    expect(await read()).toEqual([refused])
    // Mended, the node says nothing more, and the report says so too.
    await send(a, beat(a, 's', 2, [report('w-issues', 1, { state: 'running' })]))
    expect(await read()).toBeUndefined()
  })

  test('an unchanged report writes nothing', async () => {
    const a = await enrollNode(db, ca, 'quiet')
    const steady = report('w-quiet', 1, { changedAt: '2026-01-01T00:00:00Z' })
    await send(a, beat(a, 's', 1, [steady]))
    const [before] = await rows<{ observed_at: Date }>(
      db,
      sql`SELECT observed_at FROM fleet_observations WHERE workload = 'w-quiet'`,
    )
    await Bun.sleep(20)
    await send(a, beat(a, 's', 2, [steady]))
    const [after] = await rows<{ observed_at: Date }>(
      db,
      sql`SELECT observed_at FROM fleet_observations WHERE workload = 'w-quiet'`,
    )
    expect(after?.observed_at).toEqual(before?.observed_at as Date)
  })

  test('a daemon restart is a new session, not a clone', async () => {
    const a = await enrollNode(db, ca, 'restart')
    await send(a, beat(a, 'one', 1))
    await send(a, beat(a, 'one', 2))
    await send(a, beat(a, 'two', 1))
    // The old daemon's last beat, still in flight: refused, but no quarantine.
    await expect(send(a, beat(a, 'one', 3))).rejects.toMatchObject({ code: 'session_replaced' })
    const summary = (await nodeSummaries(db, THRESHOLDS)).find((n) => n.id === a.id)
    expect(summary?.duplicate_seen_at).toBeNull()
    expect((await events('node.daemon_restarted')).some((e) => e.node_id === a.id)).toBe(true)
  })

  test('a second host with the same identity is caught and quarantined', async () => {
    const a = await enrollNode(db, ca, 'clone')
    for (let seq = 1; seq <= 5; seq++) await send(a, beat(a, 'original', seq))
    // A copy of the disk has been beating on its own for a while.
    await expect(send(a, beat(a, 'clone', 40))).rejects.toMatchObject({ code: 'duplicate_identity' })
    const summary = (await nodeSummaries(db, THRESHOLDS)).find((n) => n.id === a.id)
    expect(summary?.duplicate_seen_at).not.toBeNull()
    expect((await events('node.duplicate_identity')).filter((e) => e.node_id === a.id)).toHaveLength(1)
    // The original keeps reporting, and gets nothing new placed on it until an operator decides.
    expect((await send(a, beat(a, 'original', 6))).lifecycle).toBe('active')
  })

  test('a node that moves to a new address is reached there, and the change is recorded', async () => {
    const a = await enrollNode(db, ca, 'moving', { apiAddress: '10.0.0.5:7443' })
    await send(
      a,
      beat(a, 's', 1, [], { addresses: { api: '10.0.9.5:7443', edge: '10.0.9.5', control: '10.0.9.5' } }),
    )
    const summary = (await nodeSummaries(db, THRESHOLDS)).find((n) => n.id === a.id)
    expect([summary?.api_address, summary?.edge_host, summary?.control_host]).toEqual([
      '10.0.9.5:7443',
      '10.0.9.5',
      '10.0.9.5',
    ])
    expect((await events('node.addresses_changed')).some((e) => e.node_id === a.id)).toBe(true)
    // Something that isn't an address is ignored, not stored.
    await send(a, beat(a, 's', 2, [], { addresses: { api: 'http://x/?', edge: 'a b', control: '10.0.9.5' } }))
    expect((await nodeSummaries(db, THRESHOLDS)).find((n) => n.id === a.id)?.api_address).toBe(
      '10.0.9.5:7443',
    )
  })

  test('renewal: the new certificate works alongside the old until a heartbeat brings it', async () => {
    const a = await enrollNode(db, ca, 'renew')
    await send(a, beat(a, 's', 1))
    // Due: the current certificate ends within the renewal window.
    await db.execute(
      sql`UPDATE fleet_nodes SET cert_expires_at = now() + interval '5 days' WHERE id = ${a.id}`,
    )
    expect((await send(a, beat(a, 's', 2))).renew).toBe(true)
    const key = await nodeKey()
    const renewed = await renew(db, ca, registryOptions(), a.id, a.sha, { nodeId: a.id, csrPem: key.csrPem })
    const next = { id: a.id, sha: pemSha256(renewed.clientCertPem) }
    // The old one still works: a renewal the node never received doesn't lock it out.
    expect((await send(a, beat(a, 's', 3))).lifecycle).toBe('active')
    // The first heartbeat with the new one makes it the only one.
    expect((await send(next, beat(a, 's', 4))).renew).toBeUndefined()
    await expect(send(a, beat(a, 's', 5))).rejects.toMatchObject({ code: 'certificate_not_current' })
    expect((await events('node.certificate_renewed')).some((e) => e.node_id === a.id)).toBe(true)
  })

  test('a certificate is due for renewal by the database’s clock, whatever this process’s says', async () => {
    const a = await enrollNode(db, ca, 'renew-clock')
    await send(a, beat(a, 's', 1))
    const left = (days: number) =>
      db.execute(
        sql`UPDATE fleet_nodes SET cert_expires_at = now() + make_interval(days => ${days}) WHERE id = ${a.id}`,
      )
    const now = Date.now()
    try {
      // Three weeks fast: 25 days left is not yet within the 10 days before the end.
      await left(25)
      setSystemTime(new Date(now + 21 * DAY_MS))
      expect((await send(a, beat(a, 's', 2))).renew).toBeUndefined()
      // Three weeks slow: 5 days left is.
      await left(5)
      setSystemTime(new Date(now - 21 * DAY_MS))
      expect((await send(a, beat(a, 's', 3))).renew).toBe(true)
    } finally {
      setSystemTime()
    }
  })

  test("renewal needs the node's current certificate, and a node in good standing", async () => {
    const a = await enrollNode(db, ca, 'renew-refused')
    const key = await nodeKey()
    await expect(
      renew(db, ca, registryOptions(), a.id, 'f'.repeat(64), { nodeId: a.id, csrPem: key.csrPem }),
    ).rejects.toMatchObject({
      code: 'certificate_not_current',
    })
    await db.execute(sql`UPDATE fleet_nodes SET duplicate_seen_at = now() WHERE id = ${a.id}`)
    await expect(
      renew(db, ca, registryOptions(), a.id, a.sha, { nodeId: a.id, csrPem: key.csrPem }),
    ).rejects.toMatchObject({
      code: 'quarantined',
    })
  })

  test('a node enrolls again under its own id, and its old certificate stops working', async () => {
    const a = await enrollNode(db, ca, 'reenroll')
    await place('w-reenroll', a.id, 5)
    await send(a, beat(a, 'before', 1, [report('w-reenroll', 5)]))
    const { token } = await createToken(db, { regionKey: '', ttlSeconds: 60, createdBy: 'test', node: a.id })
    const key = await nodeKey()
    const again = await enroll(db, ca, registryOptions(), {
      token,
      csrPem: key.csrPem,
      facts: facts('reenroll'),
    })
    expect(again.nodeId).toBe(a.id)
    // The old certificate is refused; the new one beats, in a new session, keeping the placement.
    await expect(send(a, beat(a, 'before', 2))).rejects.toMatchObject({ code: 'certificate_not_current' })
    const renewed = { id: a.id, sha: pemSha256(again.clientCertPem) }
    const answer = await send(renewed, beat(renewed, 'after', 1, [report('w-reenroll', 5)]))
    expect(answer.fences).toEqual([])
    const [placement] = await rows<{ node_id: string }>(
      db,
      sql`SELECT node_id FROM fleet_placements WHERE workload = 'w-reenroll'`,
    )
    expect(placement?.node_id).toBe(a.id)
    expect((await events('node.reenrolled')).map((e) => e.node_id)).toContain(a.id)
    // Once: another key can't use it again.
    await expect(
      enroll(db, ca, registryOptions(), {
        token,
        csrPem: (await nodeKey()).csrPem,
        facts: facts('reenroll'),
      }),
    ).rejects.toMatchObject({ code: 'invalid_token' })
  })

  test('loss is confirmed, never timed; a returning copy is fenced, gets no lease, and is recorded as a fork', async () => {
    const a = await enrollNode(db, ca, 'lost-a')
    const b = await enrollNode(db, ca, 'lost-b')
    await send(a, beat(a, 's', 1, [report('w-lost', 1)]))
    await place('w-lost', a.id, 1)
    // It is beating: an operator can't declare it lost without forcing.
    await expect(
      confirmLost(db, THRESHOLDS, a.id, { fencedBy: 'test', reason: 'test', by: 'test' }),
    ).rejects.toBeInstanceOf(Refused)
    await silence(a.id)
    const lost = await confirmLost(db, THRESHOLDS, a.id, {
      fencedBy: 'powered off (test)',
      reason: 'disk died',
      by: 'test',
    })
    expect(lost.displaced).toEqual(['w-lost'])
    // Rebuilt elsewhere from a snapshot: epoch 1 ended because its host was lost.
    await db.execute(
      sql`UPDATE fleet_placement_history SET ended_at = now(), end_reason = 'lost' WHERE workload = 'w-lost'`,
    )
    await place('w-lost', b.id, 2)
    // The lost node comes back, still holding epoch 1, in a new session that counts once it can.
    await send(a, beat(a, 's2', 1, [report('w-lost', 1)])).then(() => pastHold(a.id))
    const back = await send(a, beat(a, 's2', 2, [report('w-lost', 1)]))
    expect([back.lifecycle, back.leaseSeconds]).toEqual(['lost', 0])
    expect(back.fences).toEqual([{ workload: 'w-lost', currentEpoch: 2 }])
    expect((await events('fork.detected')).some((e) => e.workload === 'w-lost')).toBe(true)
    expect((await events('node.returned_while_lost')).some((e) => e.node_id === a.id)).toBe(true)
    // A lost node's certificates are not renewed.
    const key = await nodeKey()
    await expect(
      renew(db, ca, registryOptions(), a.id, a.sha, { nodeId: a.id, csrPem: key.csrPem }),
    ).rejects.toMatchObject({
      code: 'lost',
    })
  })

  test('a node that comes back before its servers were rebuilt can be reinstated', async () => {
    const a = await enrollNode(db, ca, 'reinstate')
    await place('w-back', a.id, 1)
    await silence(a.id)
    await confirmLost(db, THRESHOLDS, a.id, { fencedBy: 'test', reason: 'test', by: 'test' })
    expect((await reinstate(db, a.id, 'test')).restored).toEqual(['w-back'])
    const [placement] = await rows<{ state: string }>(
      db,
      sql`SELECT state FROM fleet_placements WHERE workload = 'w-back'`,
    )
    expect(placement?.state).toBe('placed')
  })

  test('retiring needs an empty node, and its certificates stop working', async () => {
    const a = await enrollNode(db, ca, 'retire')
    await place('w-retire', a.id, 1)
    await expect(retire(db, a.id, 'test')).rejects.toMatchObject({ code: 'not_empty' })
    await db.execute(
      sql`UPDATE fleet_placements SET state = 'released', node_id = NULL WHERE workload = 'w-retire'`,
    )
    await retire(db, a.id, 'test')
    await expect(send(a, beat(a, 's', 1))).rejects.toMatchObject({ code: 'retired' })
  })

  test('draining is an operator’s, and recorded with who asked', async () => {
    const a = await enrollNode(db, ca, 'drain')
    await drain(db, a.id, 'operator:ada')
    await expect(drain(db, a.id, 'operator:ada')).rejects.toMatchObject({ code: 'wrong_lifecycle' })
    expect((await events('node.draining')).find((e) => e.node_id === a.id)?.data.by).toBe('operator:ada')
  })

  test('the ledger counts what placements promised on each node', async () => {
    const a = await enrollNode(db, ca, 'ledger')
    await place('w-l1', a.id, 1)
    await place('w-l2', a.id, 1, 'placing')
    await place('w-l3', a.id, 1, 'displaced')
    const summary = (await nodeSummaries(db, THRESHOLDS)).find((n) => n.id === a.id)
    expect(summary?.allocated).toEqual({ memoryMb: 2048, cpuMillis: 500, diskGb: 6, workloads: 2 })
  })

  test('only servers that run, or are being started or made, hold memory', async () => {
    const a = await enrollNode(db, ca, 'admission')
    /** The last power command, `ago` seconds back. */
    const power = (workload: string, desired: 'running' | 'stopped', ago: number) =>
      db.execute(sql`
        UPDATE fleet_placements SET desired_power = ${desired}, power_changed_at = now() - make_interval(secs => ${ago})
        WHERE workload = ${workload}`)
    /** What the node reported of the copy, changed `ago` seconds back. */
    const observe = (workload: string, state: string, ago: number) =>
      db.execute(sql`
        INSERT INTO fleet_observations (node_id, workload, epoch, state, report, state_changed_at)
        VALUES (${a.id}, ${workload}, 1, ${state}, '{}'::jsonb, now() - make_interval(secs => ${ago}))`)
    // Asleep: told to stop, and stopped.
    await place('w-h1', a.id, 1)
    await power('w-h1', 'stopped', 600)
    await observe('w-h1', 'stopped', 590)
    // Being made.
    await place('w-h2', a.id, 1, 'placing')
    // Told to start; the node hasn't reported since.
    await place('w-h3', a.id, 1)
    await power('w-h3', 'running', 60)
    // Running.
    await place('w-h4', a.id, 1)
    await power('w-h4', 'running', 600)
    await observe('w-h4', 'running', 590)
    // Crashed after it was started: it holds nothing until it is started again.
    await place('w-h5', a.id, 1)
    await power('w-h5', 'running', 600)
    await observe('w-h5', 'crashed', 10)
    // Started just now; the node last reported it stopped, before the start.
    await place('w-h6', a.id, 1)
    await power('w-h6', 'running', 5)
    await observe('w-h6', 'stopped', 600)
    // Told to stop, but the node still runs it.
    await place('w-h7', a.id, 1)
    await power('w-h7', 'stopped', 60)
    await observe('w-h7', 'running', 600)
    // Made and told to start; the node reports it made, not yet started.
    await place('w-h8', a.id, 1)
    await power('w-h8', 'running', 60)
    await observe('w-h8', 'created', 10)
    // Its host was lost: it holds disk on the node, nothing else.
    await place('w-h9', a.id, 1, 'displaced')
    const summary = (await nodeSummaries(db, THRESHOLDS)).find((n) => n.id === a.id)
    expect(summary?.running).toEqual({ memoryMb: 6 * 1024, cpuMillis: 6 * 250, workloads: 6 })
    expect(summary?.allocated).toEqual({
      memoryMb: 8 * 1024,
      cpuMillis: 8 * 250,
      diskGb: 9 * 2,
      workloads: 8,
    })
    const holding = await Promise.all(
      ['w-h1', 'w-h2', 'w-h5', 'w-h6', 'w-h7', 'w-h9'].map((w) => holdsMemory(db, w)),
    )
    expect(holding).toEqual([false, true, false, true, true, false])
  })

  test("silence while no endpoint listened isn't held against a node", async () => {
    const a: EnrolledNode = await enrollNode(db, ca, 'outage')
    await send(a, beat(a, 's', 1))
    await silence(a.id)
    expect((await nodeSummaries(db, THRESHOLDS)).find((n) => n.id === a.id)?.derivedHealth).toBe(
      'unavailable',
    )
    // Every endpoint went away; one just came back.
    await db.execute(sql`DELETE FROM fleet_endpoints`)
    await endpointSeen(db, 'fresh', new Date())
    expect(await listeningSeconds(db)).toBeLessThan(5)
    expect((await nodeSummaries(db, THRESHOLDS)).find((n) => n.id === a.id)?.derivedHealth).toBe('suspect')
  })

  test('sessions: a stale sequence, a restart and a clone are told apart', () => {
    const node = { session_id: 'a', heartbeat_seq: '10', previous_session_id: null, session_age: 600, age: 2 }
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
})

describe.skipIf(!hasDatabase)('join tokens, against Postgres', () => {
  beforeAll(async () => {
    ;({ db, drop } = await freshDatabase())
    ca = await testCa()
  })
  afterAll(() => drop?.())

  test('a token is for a fleet region the region map names, and nothing else', async () => {
    const regions = ['fsn1', 'ash']
    const refused = createToken(db, { regionKey: 'eu', ttlSeconds: 60, createdBy: 'test', regions })
    await expect(refused).rejects.toMatchObject({ code: 'unknown_region', status: 400 })
    await expect(refused).rejects.toThrow('FLEET_REGION_MAP maps to fsn1, ash')
    await createToken(db, { regionKey: 'ash', ttlSeconds: 60, createdBy: 'test', regions })
    // One that re-enrolls a node takes the node's region, whatever it was given.
    const a = await enrollNode(db, ca, 'region-kept')
    await createToken(db, { regionKey: '', ttlSeconds: 60, createdBy: 'test', node: a.id, regions })
  })

  test('a pasted token names the endpoint, the CA and the deployment, and enrolls by its secret', async () => {
    const { token } = await createToken(db, { regionKey: 'eu', ttlSeconds: 60, createdBy: 'test' })
    const join = { url: 'https://10.0.0.2:8443', caPem: ca.pem, deployment: 'test' }
    const pasted = joinToken(join, token)
    expect(pasted.startsWith('bk1.')).toBe(true)
    expect(JSON.parse(Buffer.from(pasted.slice(4), 'base64url').toString())).toEqual({
      u: 'https://10.0.0.2:8443',
      h: caSha256(ca.pem),
      d: 'test',
      s: token,
    })
    // blocklyd sends the secret alone; one that sends the whole token (an older one) enrolls too.
    const key = await nodeKey()
    const enrolled = await enroll(db, ca, registryOptions(), {
      token: pasted,
      csrPem: key.csrPem,
      facts: facts('j1'),
    })
    const again = await enroll(db, ca, registryOptions(), { token, csrPem: key.csrPem, facts: facts('j1') })
    expect(again.nodeId).toBe(enrolled.nodeId)
    await expect(
      enroll(db, ca, registryOptions(), { token: 'bk1.e30', csrPem: key.csrPem, facts: facts('j2') }),
    ).rejects.toThrow('not valid')
  })
})

describe.skipIf(!hasDatabase)("the ledger and a node's own answers, against Postgres", () => {
  beforeAll(async () => {
    ;({ db, drop } = await freshDatabase())
    ca = await testCa()
  })
  afterAll(() => drop?.())

  test("a suspended copy holds its memory; one the node's own answer says stopped frees it at once", async () => {
    const a = await enrollNode(db, ca, 'answers')
    const power = (workload: string, desired: 'running' | 'stopped') =>
      db.execute(sql`
        UPDATE fleet_placements SET desired_power = ${desired}, power_changed_at = now() WHERE workload = ${workload}`)
    const observe = (workload: string, state: string) =>
      db.execute(sql`
        INSERT INTO fleet_observations (node_id, workload, epoch, state, report, state_changed_at)
        VALUES (${a.id}, ${workload}, 1, ${state}, ${JSON.stringify({ id: workload, state })}::jsonb,
                now() - interval '10 minutes')`)
    /** The node's answer to a call about the copy, as far as the ledger reads it. */
    const view = (id: string, epoch: number, state: WorkloadView['state']) =>
      ({ id, epoch, state }) as WorkloadView
    // Asleep with its memory kept.
    await place('w-s1', a.id, 1)
    await power('w-s1', 'stopped')
    await observe('w-s1', 'suspended')
    // Running, as its node last reported it.
    await place('w-s2', a.id, 1)
    await power('w-s2', 'running')
    await observe('w-s2', 'running')
    expect(await holdsMemory(db, 'w-s1')).toBe(true)
    expect(await holdsMemory(db, 'w-s2')).toBe(true)
    // An answer about another epoch's copy says nothing of this one.
    expect(await answered(db, a.id, view('w-s2', 2, 'stopped'))).toBe(false)
    expect(await holdsMemory(db, 'w-s2')).toBe(true)
    // Stopped: the stop's answer counts before the node beats again.
    await power('w-s2', 'stopped')
    expect(await answered(db, a.id, view('w-s2', 1, 'stopped'))).toBe(true)
    expect(await holdsMemory(db, 'w-s2')).toBe(false)
    const [observed] = await rows<{ state: string; report: unknown }>(
      db,
      sql`SELECT state, report FROM fleet_observations WHERE workload = 'w-s2'`,
    )
    expect(observed).toEqual({ state: 'stopped', report: { id: 'w-s2', state: 'stopped' } })
    // The same answer again changes nothing.
    expect(await answered(db, a.id, view('w-s2', 1, 'stopped'))).toBe(false)
    // A start whose answer says the copy didn't stay up gives its claim back too.
    await power('w-s2', 'running')
    expect(await holdsMemory(db, 'w-s2')).toBe(true)
    expect(await answered(db, a.id, view('w-s2', 1, 'crashed'))).toBe(true)
    expect(await holdsMemory(db, 'w-s2')).toBe(false)
    const summary = (await nodeSummaries(db, THRESHOLDS)).find((n) => n.id === a.id)
    expect(summary?.running).toEqual({ memoryMb: 1024, cpuMillis: 250, workloads: 1 })
  })
})
