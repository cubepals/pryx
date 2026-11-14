// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** A real control and local pryxd deployment for public API, CLI and edge CI suites. */
import { spawnSync } from 'node:child_process'
import { randomBytes } from 'node:crypto'
import { existsSync, mkdirSync, readFileSync, rmSync } from 'node:fs'
import type { AddressInfo } from 'node:net'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { startEdge } from '../../edge/src/agent.ts'
import { edgeConfig } from '../../edge/src/config.ts'
import { FleetCa } from '../src/ca.ts'
import { loadConfig } from '../src/config.ts'
import { SECOND_MS } from '../src/durations.ts'
import { createKey } from '../src/keys.ts'
import { startControl } from '../src/main.ts'
import { nodeSummaries } from '../src/registry.ts'
import { freshDatabase, hasDatabase } from '../test/nodes.ts'
import { spawnNodes } from './deployment-nodes.ts'
import {
  nativeEdgePortRange,
  nativeNetworkName,
  TIER_BENCH_CPU_OVERCOMMIT,
  TIER_BENCH_UNIT,
} from './runtime-config.ts'
import { createTestBucket } from './test-bucket.ts'
import { freePort, until } from './waits.ts'
export const bin = process.env.PRYXD_BIN ?? new URL('../../target/debug/pryxd', import.meta.url).pathname
export const ready = Boolean(
  existsSync(bin) && hasDatabase && process.env.S3_TEST_ENDPOINT && existsSync('/var/run/docker.sock'),
)
export const LEASE_TEST = { seconds: 10, renewSeconds: 3 }
export async function deployment(
  options: {
    localPreviews?: boolean
    repoImage?: string
    webhookAllowHosts?: string[]
    egressPolicy?: boolean
    nativeTools?: boolean
    allowedImages?: string[]
    storeLease?: boolean
    nodes?: number
    runtimeEndpoint?: string
    binary?: string
    tier?: import('../src/isolation.ts').Isolation
  } = {},
) {
  const name = `public${randomBytes(4).toString('hex')}`,
    dir = join(tmpdir(), name)
  mkdirSync(dir, { recursive: true })
  const fresh = await freshDatabase(),
    pems = await FleetCa.generate(name)
  const s3 = {
    endpoint: process.env.S3_TEST_ENDPOINT ?? '',
    bucket: name,
    accessKeyId: process.env.S3_TEST_ACCESS_KEY_ID ?? 'blockly-test',
    secretAccessKey: process.env.S3_TEST_SECRET_ACCESS_KEY ?? 'blockly-test-secret',
  }
  await createTestBucket(s3.endpoint, s3, s3.bucket)
  const operatorToken = randomBytes(32).toString('hex'),
    endpointPort = await freePort()
  const config = loadConfig({
    DATABASE_URL: fresh.url,
    PRYX_DEPLOYMENT: name,
    PRYX_DEFAULT_ISOLATION: options.tier,
    PRYX_REPO_IMAGE: options.repoImage,
    ...(options.nativeTools ? { PRYX_EDGE_PORT_RANGE: nativeEdgePortRange().join('-') } : {}),
    PRYX_UNIT_MEMORY_MB: process.env.PRYX_TIER_BENCH === '1' ? String(TIER_BENCH_UNIT.memoryMb) : undefined,
    PRYX_UNIT_CPU_MILLIS: process.env.PRYX_TIER_BENCH === '1' ? String(TIER_BENCH_UNIT.cpuMillis) : undefined,
    PRYX_CPU_OVERCOMMIT: process.env.PRYX_TIER_BENCH === '1' ? String(TIER_BENCH_CPU_OVERCOMMIT) : undefined,
    PRYX_STORE_LEASE: options.storeLease ? 'on' : 'off',
    PRYX_STORE_LEASE_SECONDS: String(LEASE_TEST.seconds),
    PRYX_STORE_LEASE_RENEW_SECONDS: String(LEASE_TEST.renewSeconds),
    PRYX_CA_CERT: pems.certPem,
    PRYX_CA_KEY: pems.keyPem,
    PRYX_OPERATOR_TOKEN: operatorToken,
    PRYX_EDGE_TOKEN: randomBytes(32).toString('hex'),
    PRYX_NODE_LISTEN: `127.0.0.1:${endpointPort}`,
    S3_ENDPOINT: s3.endpoint,
    S3_RUNTIME_ENDPOINT: options.runtimeEndpoint,
    S3_BUCKET: s3.bucket,
    S3_ACCESS_KEY: s3.accessKeyId,
    S3_SECRET_KEY: s3.secretAccessKey,
    PRYX_WEBHOOK_ALLOW_HOSTS: options.webhookAllowHosts?.join(','),
    PRYX_PUBLIC_DOMAIN: 'pryx.test',
  })
  const control = await startControl({
    ...config,
    apiListen: '127.0.0.1:0',
    operatorListen: '127.0.0.1:0',
    heartbeatSeconds: 1,
    suspend: true,
  })
  const edge = await startEdge({
    ...edgeConfig({
      PRYX_CONTROL_URL: control.apiUrl,
      PRYX_EDGE_TOKEN: control.config.edgeToken,
      PRYX_EDGE_HOST: '127.0.0.1',
      PRYX_EDGE_ID: `${name}-fixture`,
    }),
    httpPort: 0,
    tlsPort: 0,
    metricsPort: 0,
  })
  const edgePort = (edge.listeners.get('http:80')?.address() as AddressInfo).port
  if (!control.edge.agents) throw new Error('Missing agent capability service')
  control.edge.agents.localOrigin = `http://127.0.0.1:${edgePort}`
  const count = options.nodes ?? 1
  const children = await spawnNodes({
    name,
    dir,
    bin: options.binary ?? bin,
    options: {
      ...options,
      ...(options.nativeTools || options.localPreviews
        ? { agentPublicOrigin: `http://127.0.0.1:${edgePort}` }
        : {}),
    },
    pems,
    control,
    config,
  })
  try {
    await until('public node healthy', async () => {
      const nodes = await nodeSummaries(control.db, { suspectSeconds: 15, unavailableSeconds: 45 })
      return nodes.length === count && nodes.every((n) => n.derivedHealth === 'healthy') ? nodes : null
    })
  } catch (error) {
    for (const { path, child } of children) {
      const log = readFileSync(join(path, '..', 'pryxd.log'), 'utf8')
      console.error(
        'NODE_STARTUP_FAILURE',
        child.exitCode,
        log
          .split('\n')
          .filter((line) => /Error:|ERROR|invalid configuration/i.test(line))
          .slice(-10)
          .join('\n'),
      )
    }
    throw error
  }
  await control.runtime.refreshRegistry()
  const credential = await createKey(control.db, 'default', 'e2e')
  return {
    control,
    edge,
    dir,
    credential,
    children,
    env: {
      ...process.env,
      PRYX_API_URL: control.apiUrl.replace(/\/$/, ''),
      PRYX_API_KEY: credential.key,
      PRYX_OPERATOR_API: control.operatorUrl.replace(/\/$/, ''),
      PRYX_OPERATOR_TOKEN: operatorToken,
      PRYX_EDGE_TOKEN: control.config.edgeToken,
    },
    close: async () => {
      for (const { child, path } of children) {
        const kill = ['pkill', '-TERM', '-f', '--', `serve --config ${path}`]
        spawnSync(
          process.env.PRYXD_SUDO === '1' ? 'sudo' : (kill[0] ?? 'pkill'),
          process.env.PRYXD_SUDO === '1' ? ['-n', ...kill] : kill.slice(1),
        )
        let timer: ReturnType<typeof setTimeout> | undefined
        const exited = await Promise.race([
          child.exited.then(() => true),
          new Promise<false>((resolve) => {
            timer = setTimeout(() => resolve(false), 10 * SECOND_MS)
          }),
        ])
        clearTimeout(timer)
        if (!exited) {
          // Fault suites deliberately suspend nodes in the middle of runtime requests. Cleanup
          // cannot depend on that damaged node completing another request or a graceful drain.
          const force = ['pkill', '-KILL', '-f', '--', `serve --config ${path}`]
          spawnSync(
            process.env.PRYXD_SUDO === '1' ? 'sudo' : (force[0] ?? 'pkill'),
            process.env.PRYXD_SUDO === '1' ? ['-n', ...force] : force.slice(1),
          )
          await child.exited
        }
      }
      const ids = spawnSync('docker', ['ps', '-aq', '--filter', `label=blocklyd.deployment=${name}`])
        .stdout.toString()
        .split(/\s+/)
        .filter(Boolean)
      if (ids.length) spawnSync('docker', ['rm', '-f', ...ids])
      await edge.stop()
      await control.stop()
      await fresh.drop()
      if (process.env.PRYXD_SUDO === '1') spawnSync('sudo', ['-n', 'rm', '-rf', dir])
      else rmSync(dir, { recursive: true, force: true })
      if (options.nativeTools) {
        const removed = spawnSync('docker', ['network', 'rm', nativeNetworkName(name)])
        if (removed.status !== 0) throw new Error('Native fixture bridge cleanup failed')
      }
    },
  }
}
