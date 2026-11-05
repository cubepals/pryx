// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** A real Postgres API with the existing node double, isolated per test module. */
import type { Logger } from 'drizzle-orm/logger'
import { FleetStore } from '../src/archive-transport.ts'
import { createPool } from '../src/database.ts'
import { THRESHOLDS } from '../src/health.ts'
import { createKey } from '../src/keys.ts'
import type { NodeClient } from '../src/node-client.ts'
import { DEFAULTS } from '../src/placement.ts'
import { createPublicApi } from '../src/public-api.ts'
import { heartbeat } from '../src/registry.ts'
import { FleetRuntime } from '../src/runtime-engine.ts'
import { Sandboxes } from '../src/sandboxes.ts'
import { FlakyStore, Nodes } from './flow-nodes.ts'
import { beat, enrollNode, freshDatabase, registryOptions, testCa } from './nodes.ts'
export async function publicContext(logger?: Logger) {
  const fresh = await freshDatabase(logger),
    locks = createPool(fresh.url),
    store = new FlakyStore()
  await store.start()
  const nodes = new Nodes(store),
    client = nodes as unknown as NodeClient
  const engine = new FleetRuntime({
    db: fresh.db,
    nodes: client,
    store: new FleetStore(store, 'test'),
    deployment: 'test',
    regionMap: {},
    placement: DEFAULTS,
    cpuMillisPerGb: 250,
    thresholds: THRESHOLDS,
  })
  const ca = await testCa(),
    node = await enrollNode(fresh.db, ca, 'public-node', { region: 'default' })
  await heartbeat(
    fresh.db,
    registryOptions(),
    node.id,
    node.sha,
    beat(node, 'public', 1, [], {
      features: ['placement-epochs', 'data-transfer', 'local-snapshots', 'export-exclude', 'execution-lease'],
    }),
  )
  await engine.refreshRegistry()
  const boxes = new Sandboxes(fresh.db, engine, locks, client)
  const api = createPublicApi({ db: fresh.db, boxes, nodes: client, domain: 'sandboxes.test' })
  const credential = await createKey(fresh.db, 'default', 'test')
  const request = (path: string, method = 'GET', body?: unknown, key?: string) =>
    api.app.request(`http://control.test/v1${path}`, {
      method,
      headers: {
        authorization: `Bearer ${credential.key}`,
        'content-type': 'application/json',
        ...(key ? { 'Idempotency-Key': key } : {}),
      },
      ...(body ? { body: JSON.stringify(body) } : {}),
    })
  return {
    ...fresh,
    ...api,
    boxes,
    nodes,
    credential,
    request,
    close: async () => {
      await locks.end()
      await store.close()
      await fresh.drop()
    },
  }
}
