// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Exercise node CLI commands against the real operator router and Postgres. */

import { afterAll, beforeAll, describe, expect, test } from 'bun:test'
import { FleetStore } from '../../control/src/archive-transport.ts'
import { THRESHOLDS } from '../../control/src/health.ts'
import type { NodeClient } from '../../control/src/node-client.ts'
import { createOperatorApi } from '../../control/src/operator-api.ts'
import { DEFAULTS } from '../../control/src/placement.ts'
import { FleetRuntime } from '../../control/src/runtime-engine.ts'
import { MemoryStore } from '../../control/test/memory-store.ts'
import { enrollNode, freshDatabase, hasDatabase, testCa } from '../../control/test/nodes.ts'
import { node } from '../src/node.ts'

describe.skipIf(!hasDatabase)('node commands through the operator API', () => {
  let close: () => Promise<void>
  let id: string
  beforeAll(async () => {
    const fresh = await freshDatabase()
    const ca = await testCa()
    const enrolled = await enrollNode(fresh.db, ca, 'cli-node')
    id = enrolled.id
    const runtime = new FleetRuntime({
      db: fresh.db,
      nodes: {} as NodeClient,
      store: new FleetStore(new MemoryStore(), 'test'),
      deployment: 'test',
      regionMap: {},
      placement: DEFAULTS,
      cpuMillisPerGb: 250,
      thresholds: THRESHOLDS,
    })
    const token = 'operator-token-for-a-local-unit-test'
    const app = createOperatorApi({
      db: fresh.db,
      runtime,
      thresholds: THRESHOLDS,
      token,
      relocate: async () => {},
      join: { url: 'https://127.0.0.1:8443', caPem: ca.pem, deployment: 'test' },
      regions: ['eu'],
    })
    const server = Bun.serve({ hostname: '127.0.0.1', port: 0, fetch: app.fetch })
    const previous = { api: process.env.PRYX_OPERATOR_API, token: process.env.PRYX_OPERATOR_TOKEN }
    process.env.PRYX_OPERATOR_API = `http://127.0.0.1:${server.port}`
    process.env.PRYX_OPERATOR_TOKEN = token
    close = async () => {
      server.stop(true)
      if (previous.api === undefined) delete process.env.PRYX_OPERATOR_API
      else process.env.PRYX_OPERATOR_API = previous.api
      if (previous.token === undefined) delete process.env.PRYX_OPERATOR_TOKEN
      else process.env.PRYX_OPERATOR_TOKEN = previous.token
      await fresh.drop()
    }
  })
  afterAll(async () => {
    await close?.()
  })
  test('ls, token, drain and events use the control service', async () => {
    expect(await node(['ls'])).toMatchObject({
      nodes: expect.arrayContaining([expect.objectContaining({ id })]),
    })
    expect(await node(['token', 'eu'])).toMatchObject({ joinToken: expect.stringMatching(/^bk1\./) })
    expect(await node(['drain', id])).toMatchObject({ id, lifecycle: 'draining' })
    expect(await node(['events'])).toMatchObject({ events: expect.any(Array) })
  })
})
