// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Compose control's Postgres, fleet identity, node endpoint, operator API and serialized upkeep. */

import { isIP } from 'node:net'
import { migrate } from 'drizzle-orm/node-postgres/migrator'
import { FleetStore } from './archive-transport.ts'
import { Autoscale } from './autoscale.ts'
import { controlPlaneName, FleetCa } from './ca.ts'
import { type ControlConfig, loadConfig } from './config.ts'
import { createDb, createPool, migrationsFolder } from './database.ts'
import { DAY_MS, MINUTE_MS } from './durations.ts'
import { edgeApi } from './edge-api.ts'
import { EdgeService } from './edge-service.ts'
import { PRYX_VERSION } from './generated/release.ts'
import { bootstrapKey, withKeys } from './keys.ts'
import { NodeClient, splitAddress } from './node-client.ts'
import { startNodeEndpoint } from './node-endpoint.ts'
import { createOperatorApi } from './operator-api.ts'
import { withUpgrades } from './operator-upgrades.ts'
import { withProviders } from './provider-api.ts'
import { ProviderLedger } from './provider-ledger.ts'
import type { Provider } from './providers/provider.ts'
import { createProviders } from './providers.ts'
import { createPublicApi } from './public-api.ts'
import { FleetRuntime } from './runtime-engine.ts'
import { Sandboxes } from './sandboxes.ts'
import { S3ArchiveStore } from './store.ts'
import { StoreLeaseAuthority } from './store-lease.ts'
import { S3LeaseBucket } from './store-lease-bucket.ts'

export async function startControl(
  config: ControlConfig = loadConfig(),
  options: { providers?: ReadonlyMap<string, Provider> } = {},
) {
  const pool = createPool(config.database)
  const lockPool = createPool(config.database)
  const db = createDb(pool)
  await migrate(db, { migrationsFolder })
  if (config.bootstrapKey) await bootstrapKey(db, config.project, config.bootstrapKey)
  const ca = await FleetCa.fromPem(config.caCert, config.caKey)
  const clientIdentity = await ca.identity(controlPlaneName(config.deployment), 'client', 2)
  const nodes = new NodeClient({ ...clientIdentity, caPem: ca.pem }, config.deployment)
  const thresholds = {
    suspectSeconds: config.suspectSeconds,
    unavailableSeconds: config.unavailableSeconds,
  }
  let storeLease: StoreLeaseAuthority | undefined
  const runtime = new FleetRuntime({
    beforeStart: (nodeId, workload, epoch) =>
      storeLease?.beforeStart(nodeId, workload, epoch) ?? Promise.resolve(),
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
  if (config.storeLease.enabled) {
    const bucket = new S3LeaseBucket(config.s3)
    await bucket.probe(`${config.deployment}/store-leases/v1`)
    storeLease = new StoreLeaseAuthority(
      db,
      sandboxes,
      nodes,
      bucket,
      config.deployment,
      config.storeLease.seconds,
      config.storeLease.renewSeconds,
    )
  }
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
  withKeys(operator, db, config.project)
  const providers = options.providers ?? createProviders(config.providerEnv, config.deployment)
  const hosts = new ProviderLedger(
    db,
    sandboxes,
    providers,
    { url: config.joinUrl, caPem: ca.pem, deployment: config.deployment },
    config.providerSealKey,
    config.providerImages,
  )
  const scaling = new Autoscale(
    db,
    sandboxes,
    hosts,
    { ...config.autoscale, cpuMillisPerGb: config.cpuMillisPerGb },
    thresholds,
  )
  sandboxes.placement = {
    refused: (region, spec) => scaling.refused(region, spec),
    succeeded: (region) => scaling.succeeded(region),
  }
  withProviders(operator, hosts, scaling)
  const api = createPublicApi({
    db,
    boxes: sandboxes,
    nodes,
    domain: config.publicDomain,
    edgePortRange: config.edgePortRange,
  })
  const edge = new EdgeService(db, sandboxes, config.publicDomain, config.edgePortRange)
  api.app.route('/edge/v1', edgeApi(edge, config.edgeToken))
  const publicListen = splitAddress(config.apiListen)
  const publicServer = Bun.serve({
    hostname: publicListen.host,
    port: publicListen.port,
    fetch: api.app.fetch,
  })
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
      .exclusive('__upkeep__', async () => {
        await storeLease?.tick()
        await scaling.tick()
        await sandboxes.upkeep()
        await api.model.warmTemplates()
      })
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
    edge,
    sandboxes,
    hosts,
    scaling,
    storeLease,
    nodes,
    ca,
    config,
    operatorUrl: server.url.toString(),
    apiUrl: publicServer.url.toString(),
    stop: async () => {
      clearInterval(upkeep)
      clearInterval(rotation)
      await hosts.settle()
      unwatch()
      server.stop(true)
      publicServer.stop(true)
      await endpoint.stop()
      nodes.close()
      await lockPool.end()
      await pool.end()
    },
  }
}
if (import.meta.main && process.argv[2] === '--version') {
  console.log(`pryx-control ${PRYX_VERSION}`)
  process.exit(0)
}
if (import.meta.main) {
  const control = await startControl()
  for (const signal of ['SIGINT', 'SIGTERM'] as const)
    process.once(signal, () => {
      void control.stop().then(() => process.exit(0))
    })
  console.log('pryx control listening')
}
