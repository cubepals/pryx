// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Registry fixture state shared by enrollment, lifecycle and token suites. */

import 'reflect-metadata'
import { webcrypto } from 'node:crypto'
import * as x509 from '@peculiar/x509'
import { sql } from 'drizzle-orm'
import type { FleetCa } from '../src/ca.ts'
import type { Db } from '../src/database.ts'
import { heartbeat } from '../src/registry.ts'
import { rows } from '../src/sql.ts'
import type { HeartbeatRequest } from '../src/wire.ts'
import { registryOptions } from './nodes.ts'

let db: Db

let ca: FleetCa

let drop: () => Promise<void>

const send = (n: { id: string; sha: string }, request: HeartbeatRequest) =>
  heartbeat(db, registryOptions(), n.id, n.sha, request)

async function place(workload: string, nodeId: string, epoch: number, state = 'placed'): Promise<void> {
  await db.execute(sql`
    INSERT INTO fleet_placements (workload, node_id, epoch, state, region_key, memory_mb, cpu_millis, disk_gb)
    VALUES (${workload}, ${nodeId}, ${epoch}, ${state}, 'eu', 1024, 250, 2)
    ON CONFLICT (workload) DO UPDATE SET node_id = ${nodeId}, epoch = ${epoch}, state = ${state}`)
  await db.execute(sql`
    INSERT INTO fleet_placement_history (workload, epoch, node_id, reason) VALUES (${workload}, ${epoch}, ${nodeId}, 'test')
    ON CONFLICT DO NOTHING`)
}

const events = (kind: string) =>
  rows<{ workload: string; node_id: string; data: Record<string, unknown> }>(
    db,
    sql`SELECT * FROM fleet_events WHERE kind = ${kind}`,
  )

const silence = (id: string, minutes = 10) =>
  db.execute(
    sql`UPDATE fleet_nodes SET last_heartbeat_at = now() - make_interval(mins => ${minutes}) WHERE id = ${id}`,
  )

const pastHold = (id: string) =>
  db.execute(sql`UPDATE fleet_nodes SET session_started_at = now() - interval '30 seconds' WHERE id = ${id}`)

const ALG = { name: 'ECDSA', namedCurve: 'P-256', hash: 'SHA-256' }

/** Two requests for one key, as a node that keeps its key between attempts signs them. */
async function requestsForOneKey(): Promise<[string, string]> {
  const keys = (await webcrypto.subtle.generateKey(ALG, true, ['sign', 'verify'])) as CryptoKeyPair
  const request = async () =>
    (
      await x509.Pkcs10CertificateRequestGenerator.create({
        name: 'CN=node',
        keys,
        signingAlgorithm: ALG,
      })
    ).toString('pem')
  return [await request(), await request()]
}
export const registryContext = {
  get db() {
    return db
  },
  set db(value: typeof db) {
    db = value
  },
  get ca() {
    return ca
  },
  set ca(value: typeof ca) {
    ca = value
  },
  get drop() {
    return drop
  },
  set drop(value: typeof drop) {
    drop = value
  },
  get send() {
    return send
  },
  get place() {
    return place
  },
  get events() {
    return events
  },
  get silence() {
    return silence
  },
  get pastHold() {
    return pastHold
  },
  get ALG() {
    return ALG
  },
  get requestsForOneKey() {
    return requestsForOneKey
  },
}
