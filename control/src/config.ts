// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Environment configuration for one control deployment; values are validated before listeners open. */

import { z } from 'zod'
import { AUTOSCALE_DEFAULTS } from './autoscale.ts'
import { DEFAULT_EDGE_PORT_RANGE } from './edge-addresses.ts'
import { THRESHOLDS } from './health.ts'
import { splitAddress } from './node-client.ts'
import { DEFAULTS } from './placement.ts'
import { STORE_LEASE_DEFAULTS } from './store-lease.ts'

const number = (value: string | undefined, fallback: number) =>
  value === undefined ? fallback : Number(value)
const positive = z.number().int().positive()
const mapRegions = (text = ''): Record<string, string> =>
  text.startsWith('{')
    ? JSON.parse(text)
    : Object.fromEntries(
        text
          .split(',')
          .filter(Boolean)
          .map((pair) => {
            const [from, to] = pair.split(':')
            if (!from || !to) throw new Error('PRYX_REGION_MAP is source:destination pairs')
            return [from, to]
          }),
      )
const schema = z.object({
  database: z.string().min(1),
  deployment: z.string().regex(/^[a-z0-9][a-z0-9-]{0,62}$/),
  caCert: z.string().includes('BEGIN CERTIFICATE'),
  caKey: z.string().includes('PRIVATE KEY'),
  operatorToken: z.string().min(32),
  storeLease: z.object({ enabled: z.boolean(), seconds: positive, renewSeconds: positive }),
  providerEnv: z.record(z.string(), z.string()),
  providerSealKey: z.string().min(32),
  providerImages: z.array(z.string()),
  autoscale: z.object({
    enabled: z.boolean(),
    regions: z.record(z.string(), z.string()),
    memoryFloorMb: z.number().nonnegative(),
    idleSeconds: positive,
    minNodes: z.number().int().nonnegative(),
  }),
  bootstrapKey: z.string().min(32).optional(),
  nodeListen: z.string(),
  operatorListen: z.string(),
  apiListen: z.string(),
  project: z.string().min(1),
  edgeToken: z.string().min(32).optional(),
  edgePortRange: z
    .tuple([z.number().int().min(1024).max(65535), z.number().int().min(1024).max(65535)])
    .refine(([a, b]) => b >= a),
  publicDomain: z.string().regex(/^[a-zA-Z0-9.-]+$/),
  endpointHosts: z.array(z.string().min(1)).min(1),
  joinUrl: z.url().startsWith('https://'),
  pryxdBin: z.string(),
  regionMap: z.record(z.string(), z.string()),
  placement: z.object({
    policy: z.enum(['balanced', 'binpack', 'spread']),
    headroomMb: z.number().nonnegative(),
    memoryOvercommit: z.number().min(1),
    cpuOvercommit: z.number().positive(),
    cpuPressure: z.number().positive().max(1),
    diskOvercommit: z.number().positive(),
  }),
  cpuMillisPerGb: positive,
  heartbeatSeconds: positive.max(60),
  leaseSeconds: positive,
  suspectSeconds: positive,
  unavailableSeconds: positive,
  certDays: positive.min(2).max(397),
  renewDays: positive,
  suspend: z.boolean(),
  upgrades: z.boolean(),
  s3: z.object({
    endpoint: z.url(),
    runtimeEndpoint: z.url().optional(),
    bucket: z.string().min(1),
    region: z.string(),
    accessKeyId: z.string().min(1),
    secretAccessKey: z.string().min(1),
  }),
})
export type ControlConfig = z.infer<typeof schema>
export function loadConfig(env: Record<string, string | undefined> = process.env): ControlConfig {
  const nodeListen = env.PRYX_NODE_LISTEN ?? '127.0.0.1:8443'
  const hosts = (env.PRYX_ENDPOINT_HOSTS ?? '127.0.0.1,localhost').split(',').filter(Boolean)
  const config = schema.parse({
    database: env.DATABASE_URL,
    deployment: env.PRYX_DEPLOYMENT ?? 'pryx',
    caCert: env.PRYX_CA_CERT?.replaceAll('\\n', '\n'),
    caKey: env.PRYX_CA_KEY?.replaceAll('\\n', '\n'),
    operatorToken: env.PRYX_OPERATOR_TOKEN,
    storeLease: {
      enabled: env.PRYX_STORE_LEASE === 'on',
      seconds: number(env.PRYX_STORE_LEASE_SECONDS, STORE_LEASE_DEFAULTS.seconds),
      renewSeconds: number(env.PRYX_STORE_LEASE_RENEW_SECONDS, STORE_LEASE_DEFAULTS.renewSeconds),
    },
    providerEnv: Object.fromEntries(
      Object.entries(env).filter(([k, v]) => /^PRYX_(HETZNER|BOAT|MANUAL)_/.test(k) && v !== undefined),
    ),
    providerSealKey: env.PRYX_PROVIDER_SEAL_KEY ?? env.PRYX_OPERATOR_TOKEN,
    providerImages: (env.PRYX_PROVIDER_ALLOW_IMAGES ?? '').split(',').filter(Boolean),
    autoscale: {
      enabled: env.PRYX_AUTOSCALE === 'on',
      regions: JSON.parse(env.PRYX_AUTOSCALE_REGIONS ?? '{}'),
      memoryFloorMb: number(env.PRYX_REGION_MEMORY_FLOOR_MB, AUTOSCALE_DEFAULTS.memoryFloorMb),
      idleSeconds: number(env.PRYX_NODE_IDLE_SECONDS, AUTOSCALE_DEFAULTS.idleSeconds),
      minNodes: number(env.PRYX_REGION_MIN_NODES, AUTOSCALE_DEFAULTS.minNodes),
    },
    bootstrapKey: env.PRYX_BOOTSTRAP_API_KEY,
    nodeListen,
    operatorListen: env.PRYX_OPERATOR_LISTEN ?? '127.0.0.1:8081',
    apiListen: env.PRYX_API_LISTEN ?? '127.0.0.1:8082',
    project: env.PRYX_PROJECT ?? 'default',
    publicDomain: env.PRYX_PUBLIC_DOMAIN ?? 'localhost',
    edgeToken: env.PRYX_EDGE_TOKEN,
    edgePortRange: env.PRYX_EDGE_PORT_RANGE?.split('-').map(Number) ?? [...DEFAULT_EDGE_PORT_RANGE],
    endpointHosts: hosts,
    joinUrl: env.PRYX_JOIN_URL ?? `https://${hosts[0]}:${splitAddress(nodeListen).port}`,
    pryxdBin: env.PRYX_PRYXD_BIN ?? '/usr/local/bin/pryxd',
    regionMap: mapRegions(env.PRYX_REGION_MAP),
    placement: {
      policy: env.PRYX_PLACEMENT ?? DEFAULTS.policy,
      headroomMb: number(env.PRYX_HEADROOM_MB, DEFAULTS.headroomMb),
      memoryOvercommit: number(env.PRYX_MEMORY_OVERCOMMIT, DEFAULTS.memoryOvercommit),
      cpuOvercommit: number(env.PRYX_CPU_OVERCOMMIT, DEFAULTS.cpuOvercommit),
      cpuPressure: number(env.PRYX_CPU_PRESSURE, DEFAULTS.cpuPressure),
      diskOvercommit: number(env.PRYX_DISK_OVERCOMMIT, DEFAULTS.diskOvercommit),
    },
    cpuMillisPerGb: number(env.PRYX_CPU_MILLIS_PER_GB, 250),
    heartbeatSeconds: number(env.PRYX_HEARTBEAT_SECONDS, 5),
    leaseSeconds: number(env.PRYX_LEASE_SECONDS, 120),
    suspectSeconds: number(env.PRYX_SUSPECT_SECONDS, THRESHOLDS.suspectSeconds),
    unavailableSeconds: number(env.PRYX_UNAVAILABLE_SECONDS, THRESHOLDS.unavailableSeconds),
    certDays: number(env.PRYX_NODE_CERT_DAYS, 30),
    renewDays: number(env.PRYX_RENEW_DAYS, 10),
    suspend: env.PRYX_SUSPEND === 'on',
    upgrades: env.PRYX_UPGRADES !== 'off',
    s3: {
      endpoint: env.S3_ENDPOINT,
      runtimeEndpoint: env.S3_RUNTIME_ENDPOINT,
      bucket: env.S3_BUCKET,
      region: env.S3_REGION ?? 'auto',
      accessKeyId: env.S3_ACCESS_KEY,
      secretAccessKey: env.S3_SECRET_KEY,
    },
  })
  if (
    config.suspectSeconds <= 2 * config.heartbeatSeconds ||
    config.unavailableSeconds <= config.suspectSeconds ||
    config.leaseSeconds <= 3 * config.heartbeatSeconds ||
    config.renewDays >= config.certDays
  )
    throw new Error('Heartbeat, health, lease or renewal intervals are inconsistent')
  for (const name of ['nodeListen', 'operatorListen', 'apiListen'] as const) {
    const address = splitAddress(config[name])
    if (!address.host || !Number.isInteger(address.port) || address.port < 1 || address.port > 65535)
      throw new Error(`${name} is host:port`)
  }
  return config
}
