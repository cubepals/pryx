// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/**
 * The blocklyd rollout (upgrades.ts) through real heartbeats against Postgres: one node per region
 * at a time, the next once the previous beats healthy on the release, a failure or a timeout that
 * stops the region until a retry, the nodes never offered, and the operators' view of it.
 */
import { afterAll, beforeAll, describe, expect, test } from 'bun:test'
import type { Db } from '@blockly/db'
import { sql } from 'drizzle-orm'
import {
  beat,
  type EnrolledNode,
  enrollNode,
  freshDatabase,
  hasDatabase,
  registryOptions,
  testCa,
} from '../../../testing/fleet.ts'
import type { FleetCa } from '../ca.ts'
import type { FleetRuntime } from '../fleet-runtime.ts'
import { THRESHOLDS } from '../health.ts'
import { createOperatorApi } from '../operator-api.ts'
import { withUpgrades } from '../operator-upgrades.ts'
import { drain, heartbeat, isOlder, retryUpgrade, upgradeSummaries } from '../registry.ts'
import { rows } from '../sql.ts'
import type { HeartbeatRequest, UpgradeOffer } from '../wire.ts'

const RELEASE = { version: '0.3.0', sha256: 'a'.repeat(64) }
const FEATURES = ['placement-epochs', 'self-upgrade']

let db: Db
let ca: FleetCa
let drop: () => Promise<void>
const seqs = new Map<string, number>()

/** One beat from `node`, on `version`, and the upgrade its answer offers of `release`. */
async function send(
  node: EnrolledNode,
  version: string,
  over: Partial<HeartbeatRequest> = {},
  release: UpgradeOffer = RELEASE,
) {
  const seq = (seqs.get(node.id) ?? 0) + 1
  seqs.set(node.id, seq)
  const answer = await heartbeat(
    db,
    { ...registryOptions(), release },
    node.id,
    node.sha,
    beat(node, `s-${node.id}`, seq, [], { daemonVersion: version, features: FEATURES, ...over }),
  )
  return answer.upgrade
}

const events = async (node: EnrolledNode) =>
  (
    await rows<{ kind: string }>(
      db,
      sql`SELECT kind FROM fleet_events WHERE node_id = ${node.id} AND kind LIKE 'node.upgrade%' ORDER BY id`,
    )
  ).map((e) => e.kind)

test('versions compare by their numbers, and a version that is not one is never older', () => {
  expect(isOlder('0.2.0', '0.3.0')).toBe(true)
  expect(isOlder('0.10.0', '0.9.9')).toBe(false)
  expect(isOlder('0.3.0', '0.3.0')).toBe(false)
  expect(isOlder('1.0.0', '0.3.0')).toBe(false)
  expect(isOlder('dev', '0.3.0')).toBe(false)
  expect(isOlder(null, '0.3.0')).toBe(false)
})

beforeAll(async () => {
  if (!hasDatabase) return
  ;({ db, drop } = await freshDatabase())
  ca = await testCa()
})
afterAll(() => drop?.())

describe.skipIf(!hasDatabase)('the blocklyd rollout, against Postgres', () => {
  test('two nodes in a region are never offered at once; the next is once the first beats healthy', async () => {
    const [a, b] = [
      await enrollNode(db, ca, 'up-a', { region: 'r1' }),
      await enrollNode(db, ca, 'up-b', { region: 'r1' }),
    ]
    const elsewhere = await enrollNode(db, ca, 'up-c', { region: 'r2' })
    expect(await send(a, '0.2.0')).toEqual(RELEASE)
    expect(await send(b, '0.2.0')).toBeUndefined()
    // Another region rolls out on its own.
    expect(await send(elsewhere, '0.2.0')).toEqual(RELEASE)
    // Still offered while it downloads and restarts; the other still waits.
    expect(await send(a, '0.2.0')).toEqual(RELEASE)
    expect(await send(b, '0.2.0')).toBeUndefined()
    // Back on the new version but not reconciled yet: not upgraded, and b keeps waiting.
    expect(await send(a, '0.3.0', { reconciled: false })).toBeUndefined()
    expect(await send(b, '0.2.0')).toBeUndefined()
    expect(await send(a, '0.3.0')).toBeUndefined()
    expect(await events(a)).toEqual(['node.upgrade_offered', 'node.upgraded'])
    expect(await send(b, '0.2.0')).toEqual(RELEASE)
    expect(
      (await upgradeSummaries(db, RELEASE)).filter((n) => n.region === 'r1').map((n) => n.state),
    ).toEqual(['upgraded', 'offered'])
  })

  test('a failure stops the region until the node is retried', async () => {
    const [a, b] = [
      await enrollNode(db, ca, 'fail-a', { region: 'r3' }),
      await enrollNode(db, ca, 'fail-b', { region: 'r3' }),
    ]
    expect(await send(a, '0.2.0')).toEqual(RELEASE)
    // blocklyd rolled itself back and says so.
    const failed = { upgradeFailed: { version: '0.3.0', reason: 'exited with status 1 before it came up' } }
    expect(await send(a, '0.2.0', failed)).toBeUndefined()
    expect(await events(a)).toEqual(['node.upgrade_offered', 'node.upgrade_failed'])
    // Neither it nor the next node is offered the release again.
    expect(await send(a, '0.2.0', failed)).toBeUndefined()
    expect(await send(b, '0.2.0')).toBeUndefined()
    const [summary] = (await upgradeSummaries(db, RELEASE)).filter((n) => n.id === a.id)
    expect(summary).toMatchObject({ state: 'failed', reason: 'exited with status 1 before it came up' })

    await retryUpgrade(db, a.id, 'operator:test')
    await expect(retryUpgrade(db, a.id, 'operator:test')).rejects.toMatchObject({ code: 'not_failed' })
    expect(await send(a, '0.2.0')).toEqual(RELEASE)
    expect(await send(b, '0.2.0')).toBeUndefined()
  })

  test('an offered node that never comes back on the release fails when its time is up', async () => {
    const [a, b] = [
      await enrollNode(db, ca, 'late-a', { region: 'r4' }),
      await enrollNode(db, ca, 'late-b', { region: 'r4' }),
    ]
    expect(await send(a, '0.2.0')).toEqual(RELEASE)
    await db.execute(sql`UPDATE fleet_nodes SET upgrade_at = now() - interval '1 hour' WHERE id = ${a.id}`)
    // Gone quiet, it still holds its region.
    expect(await send(b, '0.2.0')).toBeUndefined()
    expect(await send(a, '0.2.0')).toBeUndefined()
    expect(await events(a)).toEqual(['node.upgrade_offered', 'node.upgrade_failed'])
    expect(await send(b, '0.2.0')).toBeUndefined()
  })

  test('a draining, lost, quarantined or unhealthy node is not offered, and a drained one gives its turn up', async () => {
    const [a, b] = [
      await enrollNode(db, ca, 'skip-a', { region: 'r5' }),
      await enrollNode(db, ca, 'skip-b', { region: 'r5' }),
    ]
    expect(await send(a, '0.2.0', { runtimeUp: false })).toBeUndefined()
    expect(await send(a, '0.2.0')).toEqual(RELEASE)
    await drain(db, a.id, 'operator:test')
    expect(await send(a, '0.2.0')).toBeUndefined()
    expect(await send(b, '0.2.0')).toEqual(RELEASE)
    expect((await upgradeSummaries(db, RELEASE)).find((n) => n.id === a.id)).toMatchObject({
      state: 'skipped',
      reason: 'draining',
    })
    const quarantined = await enrollNode(db, ca, 'skip-q', { region: 'r6' })
    await db.execute(sql`UPDATE fleet_nodes SET duplicate_seen_at = now() WHERE id = ${quarantined.id}`)
    expect(await send(quarantined, '0.2.0')).toBeUndefined()
  })
})

describe.skipIf(!hasDatabase)('nodes the rollout passes by, and what operators see', () => {
  test('a node too old to upgrade itself is skipped, and says so', async () => {
    const old = await enrollNode(db, ca, 'old-a', { region: 'r8' })
    expect(await send(old, '0.2.0', { features: ['placement-epochs'] })).toBeUndefined()
    expect((await upgradeSummaries(db, RELEASE)).find((n) => n.id === old.id)).toMatchObject({
      state: 'skipped',
      reason: 'it upgrades by hand: too old to upgrade itself',
    })
  })

  test('a node that takes only a signed blocklyd is offered the signature, and nothing without one', async () => {
    const signed = { ...RELEASE, signature: 'b'.repeat(128) }
    const features = [...FEATURES, 'signed-upgrade']
    const node = await enrollNode(db, ca, 'signed-a', { region: 'r10' })
    expect(await send(node, '0.2.0', { features })).toBeUndefined()
    expect((await upgradeSummaries(db, RELEASE)).find((n) => n.id === node.id)).toMatchObject({
      state: 'skipped',
      reason: 'it takes only a signed blocklyd, and this one has no blocklyd.sig beside it',
    })
    expect(await send(node, '0.2.0', { features }, signed)).toEqual(signed)
    // Still offered, signed, while it downloads and restarts.
    expect(await send(node, '0.2.0', { features }, signed)).toEqual(signed)
    // A node that doesn't list the feature is offered the release as before, without the field.
    const before = await enrollNode(db, ca, 'signed-b', { region: 'r11' })
    expect(await send(before, '0.2.0', {}, signed)).toEqual(RELEASE)
  })

  test('a node already on the release, or newer, or a deployment rolling none out, is offered nothing', async () => {
    const node = await enrollNode(db, ca, 'none-a', { region: 'r7' })
    expect(await send(node, '0.3.0')).toBeUndefined()
    expect(await send(node, '0.4.1')).toBeUndefined()
    const off = await heartbeat(
      db,
      registryOptions(),
      node.id,
      node.sha,
      beat(node, `s-${node.id}`, 99, [], {}),
    )
    expect(off.upgrade).toBeUndefined()
    expect(await events(node)).toEqual([])
  })
})

describe.skipIf(!hasDatabase)('the operators’ view', () => {
  test('lists every node in the rollout, and retries a failed one', async () => {
    const node = await enrollNode(db, ca, 'view-a', { region: 'r9' })
    await send(node, '0.2.0')
    await send(node, '0.2.0', { upgradeFailed: { version: '0.3.0', reason: 'no room' } })
    const operators = createOperatorApi({
      db,
      runtime: {} as FleetRuntime,
      thresholds: THRESHOLDS,
      token: 'operator-token',
      relocate: async () => {},
    })
    const api = withUpgrades(operators, { db, release: RELEASE })
    expect((await api.request('/fleet/v1/upgrades')).status).toBe(401)
    const headers = { authorization: 'Bearer operator-token', 'x-operator': 'test' }
    const listed = (await (await api.request('/fleet/v1/upgrades', { headers })).json()) as {
      release: unknown
      nodes: Array<{ id: string; state: string; reason: string | null }>
    }
    expect(listed.release).toEqual(RELEASE)
    expect(listed.nodes.find((n) => n.id === node.id)).toMatchObject({ state: 'failed', reason: 'no room' })
    const retried = await api.request(`/fleet/v1/nodes/${node.id}/retry-upgrade`, { method: 'POST', headers })
    expect(retried.status).toBe(200)
    expect(await send(node, '0.2.0')).toEqual(RELEASE)
  })
})
