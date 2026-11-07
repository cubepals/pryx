// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Real Postgres intents with mandatory authenticated agent peers, isolated per test module. */
import type { Logger } from 'drizzle-orm/logger'
import { agentContext } from '../../edge/test/agent-context.ts'
import { FleetStore } from '../src/archive-transport.ts'
import { createPool } from '../src/database.ts'
import { SECOND_MS } from '../src/durations.ts'
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
  const peers = new Map<string, ReturnType<typeof agentContext>>()
  const peer = (id: string) => {
    let value = peers.get(id)
    if (!value) {
      value = agentContext(id)
      peers.set(id, value)
    }
    return value
  }
  nodes.agent = (id) => {
    const manifest = peer(id).route.agent?.manifest
    if (!manifest) throw new Error('Missing peer manifest')
    return manifest
  }
  nodes.agentToken = (id) => ({
    token: peer(id).token(),
    expiresAt: Math.floor(Date.now() / SECOND_MS) + 60,
    epoch: 1,
    agent: nodes.agent?.(id),
  })
  const proxy = Bun.serve({
    hostname: '127.0.0.1',
    port: 0,
    fetch: (request) => {
      const id = new URL(request.url).pathname.split('/')[2]
      const remote = id ? peers.get(id) : undefined
      if (!remote) throw new Error('Missing peer')
      const origin = new URL(remote.url).origin
      return fetch(new Request(origin + new URL(request.url).pathname + new URL(request.url).search, request))
    },
  })
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
  let sequence = 0
  const report = () =>
    heartbeat(
      fresh.db,
      registryOptions(),
      node.id,
      node.sha,
      beat(node, 'public', ++sequence, nodes.reports(node.id), {
        features: [
          'agent',
          'files',
          'placement-epochs',
          'data-transfer',
          'local-snapshots',
          'export-exclude',
          'execution-lease',
        ],
      }),
    )
  await report()
  await engine.refreshRegistry()
  const boxes = new Sandboxes(fresh.db, engine, locks, client)
  const api = createPublicApi({ db: fresh.db, boxes, nodes: client, domain: 'sandboxes.test' })
  api.agents.localOrigin = proxy.url.origin
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
    peers,
    credential,
    request,
    report,
    close: async () => {
      proxy.stop(true)
      for (const peer of peers.values()) peer.close()
      await locks.end()
      await store.close()
      await fresh.drop()
    },
  }
}
