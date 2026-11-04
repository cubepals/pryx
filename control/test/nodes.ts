// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Fleet tests need Postgres, as the operation tests do: they run where DATABASE_URL is set. */

import 'reflect-metadata'
import { randomBytes, webcrypto } from 'node:crypto'
import * as x509 from '@peculiar/x509'
import { migrate } from 'drizzle-orm/node-postgres/migrator'
import pg from 'pg'
import { FleetCa, pemSha256 } from '../src/ca.ts'
import { createDb, createPool, type Db, migrationsFolder } from '../src/database.ts'
import { createToken, enroll, type RegistryOptions } from '../src/registry.ts'
import type { HeartbeatRequest, NodeCapacity, NodeFacts, WorkloadReport } from '../src/wire.ts'

export const hasDatabase = Boolean(process.env.DATABASE_URL)

/** A database of its own, migrated, for one test file; `drop` removes it. */
export async function freshDatabase(): Promise<{
  db: Db
  url: string
  drop: () => Promise<void>
}> {
  const base = process.env.DATABASE_URL
  if (!base) throw new Error('DATABASE_URL is not set')
  const name = `blockly_fleet_${randomBytes(5).toString('hex')}`
  const admin = new pg.Client({ connectionString: base })
  await admin.connect()
  await admin.query(`CREATE DATABASE ${name}`)
  await admin.end()
  const url = new URL(base)
  url.pathname = `/${name}`
  const pool = createPool(url.toString())
  const db = createDb(pool)
  await migrate(db, { migrationsFolder })
  return {
    db,
    url: url.toString(),
    drop: async () => {
      await pool.end()
      const dropper = new pg.Client({ connectionString: base })
      await dropper.connect()
      await dropper.query(`DROP DATABASE IF EXISTS ${name} WITH (FORCE)`)
      await dropper.end()
    },
  }
}

export async function testCa(deployment = 'test'): Promise<FleetCa> {
  const { certPem, keyPem } = await FleetCa.generate(deployment)
  return FleetCa.fromPem(certPem, keyPem)
}

const ALG = { name: 'ECDSA', namedCurve: 'P-256', hash: 'SHA-256' }

/** A node's key and a CSR for it, as pryxd makes them. */
export async function nodeKey(): Promise<{ csrPem: string; keyPem: string }> {
  const keys = (await webcrypto.subtle.generateKey(ALG, true, ['sign', 'verify'])) as CryptoKeyPair
  const csr = await x509.Pkcs10CertificateRequestGenerator.create({
    name: 'CN=node',
    keys,
    signingAlgorithm: ALG,
  })
  const der = Buffer.from(await webcrypto.subtle.exportKey('pkcs8', keys.privateKey)).toString('base64')
  return {
    csrPem: csr.toString('pem'),
    keyPem: `-----BEGIN PRIVATE KEY-----\n${der.replace(/.{64}/g, '$&\n')}\n-----END PRIVATE KEY-----\n`,
  }
}

export const registryOptions = (deployment = 'test'): RegistryOptions => ({
  deployment,
  heartbeatSeconds: 5,
  leaseSeconds: 120,
  certDays: 30,
  renewDays: 10,
})

export const capacity = (over: Partial<NodeCapacity> = {}): NodeCapacity => ({
  memoryTotalMb: 16_384,
  reservedMemoryMb: 2048,
  allocatableMemoryMb: 14_336,
  provisionedMemoryMb: 0,
  runningMemoryMb: 0,
  usedMemoryBytes: null,
  cpus: 8,
  reservedCpuMillis: 1000,
  usedCpuCores: null,
  loadAverage: null,
  diskTotalBytes: 500 * 1024 ** 3,
  diskAvailableBytes: 400 * 1024 ** 3,
  minFreeDiskMb: 2048,
  snapshotBytes: 0,
  reflink: null,
  portsTotal: 500,
  portsAllocated: 0,
  ...over,
})

export const facts = (name: string, apiAddress = '10.0.0.1:7443'): NodeFacts => ({
  hostname: name,
  bootId: `boot-${name}`,
  machineIdSha256: `machine-${name}`,
  daemonVersion: '0.2.0',
  protocol: { current: 1, supported: [1] },
  features: ['placement-epochs', 'data-transfer', 'local-snapshots', 'execution-lease'],
  apiAddress,
  edgeIps: [apiAddress.split(':')[0] ?? ''],
  controlIps: [apiAddress.split(':')[0] ?? ''],
  capacity: capacity(),
  labels: {},
})

export interface EnrolledNode {
  id: string
  /** sha256 of its client certificate, as the TLS layer would hand it to the registry. */
  sha: string
  serverCertPem: string
  clientCertPem: string
  keyPem: string
}

/** A node enrolled the way pryxd enrolls: a token, a CSR for its own key, its facts. */
export async function enrollNode(
  db: Db,
  ca: FleetCa,
  name: string,
  options: {
    region?: string
    apiAddress?: string
    deployment?: string
    labels?: Record<string, string>
    token?: string
  } = {},
): Promise<EnrolledNode> {
  const token =
    options.token ??
    (
      await createToken(db, {
        regionKey: options.region ?? 'eu',
        ttlSeconds: 60,
        createdBy: 'test',
        ...(options.labels ? { labels: options.labels } : {}),
      })
    ).token
  const key = await nodeKey()
  const enrolled = await enroll(db, ca, registryOptions(options.deployment), {
    token,
    csrPem: key.csrPem,
    facts: facts(name, options.apiAddress),
  })
  return {
    id: enrolled.nodeId,
    sha: pemSha256(enrolled.clientCertPem),
    serverCertPem: enrolled.serverCertPem,
    clientCertPem: enrolled.clientCertPem,
    keyPem: key.keyPem,
  }
}

export const report = (id: string, epoch: number, over: Partial<WorkloadReport> = {}): WorkloadReport => ({
  id,
  epoch,
  supersededBy: null,
  state: 'running',
  specDigest: 'd',
  generation: 1,
  memoryMb: 1024,
  restartCount: 0,
  exit: null,
  lastFailureAt: null,
  changedAt: new Date().toISOString(),
  ports: { game: 30000, rcon: 30001 },
  ...over,
})

export function beat(
  node: { id: string },
  session: string,
  seq: number,
  workloads: WorkloadReport[] = [],
  over: Partial<HeartbeatRequest> = {},
): HeartbeatRequest {
  return {
    nodeId: node.id,
    sessionId: session,
    bootId: 'boot',
    seq,
    daemonVersion: '0.2.0',
    protocol: { current: 1, supported: [1] },
    features: ['placement-epochs', 'data-transfer', 'local-snapshots', 'execution-lease'],
    runtimeUp: true,
    reconciled: true,
    capacity: capacity(),
    workloads,
    issues: [],
    addresses: null,
    ...over,
  }
}
