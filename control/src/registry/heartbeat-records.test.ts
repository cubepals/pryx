// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Heartbeats distinguish certificates, session boundaries and actionable refusals. */
import { afterAll, beforeAll, expect, test } from 'bun:test'
import type { Db } from '@blockly/db'
import { sql } from 'drizzle-orm'
import {
  beat,
  enrollNode,
  freshDatabase,
  hasDatabase,
  registryOptions,
  report,
  testCa,
} from '../../../testing/fleet.ts'
import type { FleetCa } from '../ca.ts'
import { rows } from '../sql.ts'
import { certificateOf, heartbeat, sessionOf } from './heartbeat.ts'
import { getNode } from './node.ts'

let db: Db
let ca: FleetCa
let drop: (() => Promise<void>) | undefined
beforeAll(async () => {
  if (!hasDatabase) return
  ;({ db, drop } = await freshDatabase())
  ca = await testCa()
})
afterAll(async () => {
  await drop?.()
})

test('session boundaries distinguish late beats, restarts and duplicate hosts', () => {
  const node: Parameters<typeof sessionOf>[0] = {
    session_id: 'current',
    previous_session_id: 'previous',
    heartbeat_seq: '10',
    session_age: 20,
    age: 0,
  }
  const next = { sessionId: 'other', seq: 4 }
  expect(sessionOf(node, { sessionId: 'current', seq: 10 })).toBe('stale')
  expect(sessionOf(node, { sessionId: 'current', seq: 11 })).toBe('current')
  expect(sessionOf(node, next)).toBe('duplicate')
  expect(sessionOf(node, { ...next, seq: 3 })).toBe('switched')
  expect(sessionOf({ ...node, age: 30 }, next)).toBe('switched')
  expect(sessionOf({ ...node, age: null }, next)).toBe('switched')
  expect(sessionOf({ ...node, session_id: null }, next)).toBe('switched')
  expect(sessionOf({ ...node, age: 29.999 }, next)).toBe('duplicate')
  expect(sessionOf({ ...node, session_age: 15 }, { sessionId: 'previous', seq: 100 })).toBe('replaced')
  expect(sessionOf({ ...node, session_age: 15.001 }, { sessionId: 'previous', seq: 100 })).toBe('duplicate')
  expect(sessionOf({ ...node, session_age: null }, { sessionId: 'previous', seq: 100 })).toBe('duplicate')
})

test.skipIf(!hasDatabase)('only a matching current or next certificate is accepted', async () => {
  const enrolled = await enrollNode(db, ca, 'certificate-records')
  const node = await getNode(db, enrolled.id)
  if (!node) throw new Error('The enrolled node must exist')
  expect(certificateOf(node, enrolled.sha)).toBe('current')
  expect(certificateOf(node, 'different-peer')).toBeNull()
  expect(certificateOf({ ...node, cert_sha256: null }, '')).toBeNull()
  expect(certificateOf({ ...node, next_cert_sha256: 'next-peer' }, 'different-peer')).toBeNull()
  expect(certificateOf({ ...node, next_cert_sha256: 'next-peer' }, 'next-peer')).toBe('next')
  expect(certificateOf({ ...node, next_cert_sha256: enrolled.sha }, enrolled.sha)).toBe('current')
})

test.skipIf(!hasDatabase)(
  'refusals retain the reason for wrong identities, certificates and retired nodes',
  async () => {
    const node = await enrollNode(db, ca, 'refusal-records')
    const request = beat(node, 'current', 1)
    await expect(
      heartbeat(db, registryOptions(), node.id, node.sha, { ...request, nodeId: 'other' }),
    ).rejects.toMatchObject({ code: 'invalid_request', message: 'the heartbeat is for another node' })
    const missing = crypto.randomUUID()
    await expect(
      heartbeat(db, registryOptions(), missing, node.sha, { ...request, nodeId: missing }),
    ).rejects.toMatchObject({ code: 'unknown_node', message: 'no such node' })
    await expect(heartbeat(db, registryOptions(), node.id, 'different-peer', request)).rejects.toMatchObject({
      code: 'certificate_not_current',
      message: "this certificate is not the node's current one",
    })
    await db.execute(sql`UPDATE fleet_nodes SET lifecycle = 'retired' WHERE id = ${node.id}`)
    await expect(heartbeat(db, registryOptions(), node.id, node.sha, request)).rejects.toMatchObject({
      code: 'retired',
      message: 'this node is retired',
    })
  },
)

test.skipIf(!hasDatabase)('invalid epochs are refused while zero remains a valid whole epoch', async () => {
  const node = await enrollNode(db, ca, 'epoch-records')
  for (const epoch of [-1, 0.5, Number.NaN, Number.POSITIVE_INFINITY, Number.MAX_SAFE_INTEGER]) {
    await expect(
      heartbeat(db, registryOptions(), node.id, node.sha, beat(node, 'current', 1, [report('copy', epoch)])),
    ).rejects.toMatchObject({
      code: 'invalid_request',
      message: expect.stringContaining('an epoch is not a whole number from 0 to '),
    })
    await expect(
      heartbeat(
        db,
        registryOptions(),
        node.id,
        node.sha,
        beat(node, 'current', 1, [report('copy', 0, { supersededBy: epoch })]),
      ),
    ).rejects.toMatchObject({ code: 'invalid_request' })
  }
  await heartbeat(db, registryOptions(), node.id, node.sha, beat(node, 'current', 1, [report('copy', 0)]))
  expect(
    await rows<{ epoch: string }>(db, sql`SELECT epoch FROM fleet_observations WHERE node_id = ${node.id}`),
  ).toEqual([{ epoch: '0' }])
})

test.skipIf(!hasDatabase)(
  'a duplicate identity is quarantined and audited once across repeated beats',
  async () => {
    const node = await enrollNode(db, ca, 'duplicate-records')
    const send = (session: string, seq: number) =>
      heartbeat(db, registryOptions(), node.id, node.sha, beat(node, session, seq))
    await send('current', 1)
    for (const seq of [4, 5])
      await expect(send('other', seq)).rejects.toMatchObject({
        code: 'duplicate_identity',
        message: 'another host is beating with this identity',
      })
    const found = await rows<{ data: unknown }>(
      db,
      sql`SELECT data FROM fleet_events WHERE node_id = ${node.id} AND kind = 'node.duplicate_identity'`,
    )
    expect(found).toEqual([{ data: { current: 'current', other: 'other', otherSeq: 4 } }])
    expect((await getNode(db, node.id))?.duplicate_seen_at).not.toBeNull()
  },
)

test.skipIf(!hasDatabase)('address changes retain their exact fields and reject invalid hosts', async () => {
  const node = await enrollNode(db, ca, 'address-records')
  const initial = await getNode(db, node.id)
  if (!initial?.api_address || !initial.edge_host || !initial.control_host)
    throw new Error('The enrolled addresses must exist')
  const original = { api: initial.api_address, edge: initial.edge_host, control: initial.control_host }
  const send = (seq: number, addresses: typeof original, bootId: string | null = initial.boot_id) =>
    heartbeat(db, registryOptions(), node.id, node.sha, beat(node, 'current', seq, [], { addresses, bootId }))
  await send(1, original)
  const changes = () =>
    rows<{ data: unknown }>(
      db,
      sql`SELECT data FROM fleet_events WHERE node_id = ${node.id} AND kind = 'node.addresses_changed' ORDER BY id`,
    )
  expect(await changes()).toEqual([])
  const control = { ...original, control: '10.0.0.90' }
  await send(2, control)
  expect((await getNode(db, node.id))?.control_host).toBe(control.control)
  expect(await changes()).toEqual([{ data: { from: original, to: control } }])
  const api = { ...control, api: '10.0.0.91:9443' }
  await send(3, api)
  const edge = { ...api, edge: '10.0.0.92' }
  await send(4, edge)
  expect(await getNode(db, node.id)).toMatchObject({
    api_address: api.api,
    edge_host: edge.edge,
    control_host: control.control,
  })
  expect(await changes()).toHaveLength(3)
  await send(5, { ...edge, edge: 'invalid/path' }, 'next-boot')
  await send(6, { ...edge, control: 'invalid/path' }, null)
  await send(7, edge, 'another-boot')
  expect(await changes()).toHaveLength(3)
  expect(await getNode(db, node.id)).toMatchObject({
    api_address: edge.api,
    edge_host: edge.edge,
    control_host: edge.control,
  })
  expect(
    await rows(db, sql`SELECT kind FROM fleet_events WHERE node_id = ${node.id} AND kind = 'node.rebooted'`),
  ).toEqual([{ kind: 'node.rebooted' }])
})

test.skipIf(!hasDatabase)('a beat from the replaced session explains the newer session', async () => {
  const node = await enrollNode(db, ca, 'replaced-records')
  const send = (session: string, seq: number) =>
    heartbeat(db, registryOptions(), node.id, node.sha, beat(node, session, seq))
  await send('first', 1)
  expect((await send('second', 1)).leaseSeconds).toBe(0)
  await expect(send('first', 2)).rejects.toMatchObject({
    code: 'session_replaced',
    message: 'a newer daemon session reports for this node',
  })
})
