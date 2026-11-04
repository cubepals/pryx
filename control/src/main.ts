// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Compose control's Postgres, fleet identity, node endpoint, operator API and serialized upkeep. */

import { isIP } from 'node:net'
import { migrate } from 'drizzle-orm/node-postgres/migrator'
import { FleetStore } from './archive-transport.ts'
import { controlPlaneName, FleetCa } from './ca.ts'
import { type ControlConfig, loadConfig } from './config.ts'
import { createDb, createPool, migrationsFolder } from './database.ts'
import { DAY_MS, MINUTE_MS } from './durations.ts'
import { NodeClient, splitAddress } from './node-client.ts'
import { startNodeEndpoint } from './node-endpoint.ts'
import { createOperatorApi } from './operator-api.ts'
import { withUpgrades } from './operator-upgrades.ts'
import { FleetRuntime } from './runtime-engine.ts'
import { Sandboxes } from './sandboxes.ts'
import { S3ArchiveStore } from './store.ts'

export async function startControl(config: ControlConfig = loadConfig()) {
  const pool = createPool(config.database)
  const lockPool = createPool(config.database)
  const db = createDb(pool)
  await migrate(db, { migrationsFolder })
  const ca = await FleetCa.fromPem(config.caCert, config.caKey)
  const clientIdentity = await ca.identity(controlPlaneName(config.deployment), 'client', 2)
  const nodes = new NodeClient({ ...clientIdentity, caPem: ca.pem }, config.deployment)
  const thresholds = {
    suspectSeconds: config.suspectSeconds,
    unavailableSeconds: config.unavailableSeconds,
  }
  const runtime = new FleetRuntime({
    db,
    nodes,
    store: new FleetStore(new S3ArchiveStore(config.s3), config.deployment),
    deployment: config.deployment,
    regionMap: config.regionMap,
    placement: config.placement,
    cpuMillisPerGb: config.cpuMillisPerGb,
    thresholds,
    suspend: config.suspend,
  })
  const sandboxes = new Sandboxes(db, runtime, lockPool, nodes)
  const unwatch = await runtime.watchRegistry()
  const endpoint = await startNodeEndpoint({
    db,
    ca,
    registry: {
      deployment: config.deployment,
      heartbeatSeconds: config.heartbeatSeconds,
      leaseSeconds: config.leaseSeconds,
      certDays: config.certDays,
      renewDays: config.renewDays,
    },
    listen: splitAddress(config.nodeListen),
    names: {
      dns: config.endpointHosts.filter((host) => !isIP(host)),
      ips: config.endpointHosts.filter((host) => Boolean(isIP(host))),
    },
    pryxdBin: config.pryxdBin,
    upgrades: config.upgrades,
  })
  const operator = createOperatorApi({
    db,
    runtime,
    thresholds,
    token: config.operatorToken,
    relocate: (id) => sandboxes.relocate(id),
    endpointStats: endpoint.stats,
    join: { url: config.joinUrl, caPem: ca.pem, deployment: config.deployment },
    regions: [...new Set(['default', ...Object.values(config.regionMap)])],
  })
  withUpgrades(operator, { db, release: endpoint.release })
  const listen = splitAddress(config.operatorListen)
  const server = Bun.serve({
    hostname: listen.host,
    port: listen.port,
    fetch: operator.fetch,
  })
  let maintaining = false
  const upkeep = setInterval(() => {
    if (maintaining) return
    maintaining = true
    sandboxes
      .exclusive('__upkeep__', () => sandboxes.upkeep())
      .catch((error) => console.error('upkeep failed', error instanceof Error ? error.name : 'error'))
      .finally(() => {
        maintaining = false
      })
  }, MINUTE_MS)
  const rotation = setInterval(() => {
    ca.identity(controlPlaneName(config.deployment), 'client', 2)
      .then((identity) => nodes.rotate({ ...identity, caPem: ca.pem }))
      .catch((error) =>
        console.error('client certificate renewal failed', error instanceof Error ? error.name : 'error'),
      )
  }, DAY_MS)
  return {
    db,
    runtime,
    sandboxes,
    nodes,
    ca,
    config,
    stop: async () => {
      clearInterval(upkeep)
      clearInterval(rotation)
      unwatch()
      server.stop(true)
      await endpoint.stop()
      nodes.close()
      await lockPool.end()
      await pool.end()
    },
  }
}
if (import.meta.main) {
  const control = await startControl()
  for (const signal of ['SIGINT', 'SIGTERM'] as const)
    process.once(signal, () => {
      void control.stop().then(() => process.exit(0))
    })
  console.log('pryx control listening')
}
