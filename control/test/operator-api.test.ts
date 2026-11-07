// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/** The operators' API as `scripts/fleet.ts` calls it, against Postgres. */

import { afterAll, beforeAll, describe, expect, test } from 'bun:test'
import { sql } from 'drizzle-orm'
import { StatusCodes } from 'http-status-codes'
import { FleetStore } from '../src/archive-transport.ts'
import type { Db } from '../src/database.ts'
import { encodeHandle } from '../src/handle.ts'
import { THRESHOLDS } from '../src/health.ts'
import { joinCommand, joinToken } from '../src/join-token.ts'
import type { NodeClient } from '../src/node-client.ts'
import { createOperatorApi } from '../src/operator-api.ts'
import { DEFAULTS } from '../src/placement.ts'
import { heartbeat } from '../src/registry.ts'
import { FleetRuntime } from '../src/runtime-engine.ts'
import { rows } from '../src/sql.ts'
import { MemoryStore } from './memory-store.ts'
import { beat, enrollNode, freshDatabase, hasDatabase, registryOptions, testCa } from './nodes.ts'

describe.skipIf(!hasDatabase)('the operator API', () => {
  let db: Db
  let drop: () => Promise<void>
  let runtime: FleetRuntime
  let api: ReturnType<typeof createOperatorApi>

  const call = (method: string, path: string, body?: object) =>
    api.fetch(
      new Request(`http://internal/fleet/v1${path}`, {
        method,
        headers: {
          authorization: 'Bearer operator-token',
          'x-operator': 'ada',
          'content-type': 'application/json',
        },
        ...(body ? { body: JSON.stringify(body) } : {}),
      }),
    )

  beforeAll(async () => {
    ;({ db, drop } = await freshDatabase())
    // Nothing here calls a node or the store: only the registry the runtime keeps is read.
    runtime = new FleetRuntime({
      db,
      nodes: {} as NodeClient,
      store: new FleetStore(new MemoryStore(), 'test'),
      deployment: 'test',
      regionMap: {},
      placement: DEFAULTS,
      cpuMillisPerGb: 250,
      thresholds: THRESHOLDS,
      log: () => {},
    })
    api = createOperatorApi({
      db,
      runtime,
      thresholds: THRESHOLDS,
      token: 'operator-token',
      relocate: async () => {},
    })
  })
  afterAll(() => drop?.())

  test("an operator corrects a node's price and its other labels, and the change is recorded", async () => {
    const ca = await testCa()
    const n = await enrollNode(db, ca, 'priced', {
      labels: { monthly_cost_cents: '4000', provider: 'hetzner' },
    })
    await heartbeat(db, registryOptions(), n.id, n.sha, beat(n, 's', 1))
    await runtime.refreshRegistry()
    const handle = encodeHandle({
      deployment: 'test',
      key: 'w-priced',
      node: n.id,
      nodeName: 'priced',
      epoch: 1,
      region: 'eu',
      edgeHost: null,
      controlHost: null,
      ports: {},
    })
    // A server of a quarter of what may be placed there: its share is a quarter of the node's month.
    const size = { memoryMb: 14_336, storageGb: 1 }
    expect(runtime.prices(handle, size)?.storageMonthCents).toBe(1000)

    const changed = await call('POST', `/nodes/${n.id}/labels`, {
      set: { monthly_cost_cents: '8000', owner: 'infra' },
      remove: ['provider'],
    })
    expect(changed.status).toBe(StatusCodes.OK)
    expect(((await changed.json()) as { labels: Record<string, string> }).labels).toEqual({
      monthly_cost_cents: '8000',
      owner: 'infra',
    })
    // The sandboxes placed there are priced from the new figure at once.
    expect(runtime.prices(handle, size)?.storageMonthCents).toBe(2000)
    const [recorded] = await rows<{ data: Record<string, unknown> }>(
      db,
      sql`SELECT data FROM fleet_events WHERE kind = 'node.labels_changed' AND node_id = ${n.id}`,
    )
    expect(recorded?.data).toEqual({
      from: { monthly_cost_cents: '4000', provider: 'hetzner' },
      to: { monthly_cost_cents: '8000', owner: 'infra' },
      by: 'operator:ada',
    })
    // A heartbeat doesn't undo it.
    await heartbeat(db, registryOptions(), n.id, n.sha, beat(n, 's', 2))
    const [node] = await rows<{ labels: Record<string, string> }>(
      db,
      sql`SELECT labels FROM fleet_nodes WHERE id = ${n.id}`,
    )
    expect(node?.labels.monthly_cost_cents).toBe('8000')
  })

  test('labels are refused when they say nothing, or give a price that is not a whole number of cents', async () => {
    const ca = await testCa()
    const n = await enrollNode(db, ca, 'refused-labels')
    const refused = async (body: object) => {
      const answer = await call('POST', `/nodes/${n.id}/labels`, body)
      return [answer.status, ((await answer.json()) as { error: { code: string } }).error.code]
    }
    expect(await refused({})).toEqual([400, 'invalid_request'])
    expect(await refused({ set: { monthly_cost_cents: '49,00' } })).toEqual([400, 'invalid_request'])
    expect(await refused({ set: { provider: 7 } })).toEqual([400, 'invalid_request'])
    expect(await refused({ set: { a: 'b' }, remove: ['a'] })).toEqual([400, 'invalid_request'])
    const unknown = await call('POST', '/nodes/00000000-0000-4000-8000-000000000000/labels', {
      set: { a: 'b' },
    })
    expect(unknown.status).toBe(StatusCodes.NOT_FOUND)
  })
})

/** `bun scripts/fleet.ts token`, against Postgres: the forms an operator is given, and what is refused. */
describe.skipIf(!hasDatabase)('tokens from the operator API', () => {
  let db: Db
  let drop: () => Promise<void>
  let api: ReturnType<typeof createOperatorApi>
  const join = { url: 'https://10.0.0.2:8443', caPem: '', deployment: 'test' }

  const mint = async (body: object) => {
    const answer = await api.fetch(
      new Request('http://internal/fleet/v1/tokens', {
        method: 'POST',
        headers: {
          authorization: 'Bearer operator-token',
          'content-type': 'application/json',
        },
        body: JSON.stringify(body),
      }),
    )
    type Minted = {
      token: string
      joinToken: string
      joinCommand: string | null
      error: { code: string }
    }
    return { status: answer.status, body: (await answer.json()) as Minted }
  }

  beforeAll(async () => {
    ;({ db, drop } = await freshDatabase())
    join.caPem = (await testCa()).pem
    api = createOperatorApi({
      db,
      runtime: {} as FleetRuntime,
      thresholds: THRESHOLDS,
      token: 'operator-token',
      relocate: async () => {},
      join,
      regions: ['fsn1'],
    })
  })
  afterAll(() => drop?.())

  test('a new node gets the bare token, the pasted form, and the line to paste it in', async () => {
    const { status, body } = await mint({
      region: 'fsn1',
      labels: { provider: 'hetzner' },
    })
    expect(status).toBe(StatusCodes.OK)
    expect(body.joinToken).toBe(joinToken(join, body.token))
    expect(body.joinCommand).toBe(`pryx node join '${body.joinToken}'`)
    expect(body.joinCommand).not.toContain('curl')
  })

  test('a region the region map doesn’t name is refused; a node re-enrolled gets its own line', async () => {
    const refused = await mint({ region: 'eu' })
    expect([refused.status, refused.body.error.code]).toEqual([StatusCodes.BAD_REQUEST, 'unknown_region'])
    const n = await enrollNode(db, await testCa(), 'reenrolled')
    const again = await mint({ node: n.id })
    expect(again.status).toBe(StatusCodes.OK)
    expect(again.body.joinToken).toBe(joinToken(join, again.body.token, n.id))
    const payload = JSON.parse(Buffer.from(again.body.joinToken.slice(4), 'base64url').toString())
    expect(payload.n).toBe(n.id)
    expect(again.body.joinCommand).toBe(joinCommand(join, again.body.joinToken))
  })
})
