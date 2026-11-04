// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/**
 * How a machine becomes a node: one-time tokens, a first enrollment, the same enrollment asked
 * again when its answer never arrived, and re-enrollment under an existing id.
 * Renewing an enrolled node's certificates lives in `renewal.ts`, and its heartbeats in
 * `heartbeat.ts`.
 */

import { randomBytes, randomUUID } from 'node:crypto'
import { sql } from 'drizzle-orm'
import { controlPlaneName, csrKeySha256, type FleetCa, type Issued } from '../ca.ts'
import type { Db, Queryable } from '../database.ts'
import { secretOf } from '../join-token.ts'
import { splitAddress } from '../node-client.ts'
import { event, jsonb, one, rows, toDate } from '../sql.ts'
import type { EnrollRequest, EnrollResponse } from '../wire.ts'
import { getNode, Refused, type RegistryOptions, sha256 } from './node.ts'

// ─── enrollment ─────────────────────────────────────────────────────────────────────────────

/**
 * A one-time token. The token itself is returned once and never stored. With `node`, it
 * re-enrolls that node: new certificates for a new key, under the same id, in the node's region.
 * Without, `regions`, when given, are the fleet regions there are (PRYX_REGION_MAP): a token
 * for another would enroll a node nothing is ever placed on.
 */
export async function createToken(
  db: Db,
  options: {
    regionKey: string
    labels?: Record<string, string>
    ttlSeconds: number
    createdBy: string
    node?: string
    regions?: readonly string[]
  },
): Promise<{ id: string; token: string; expiresAt: Date }> {
  let regionKey = options.regionKey
  if (options.node === undefined && options.regions !== undefined && !options.regions.includes(regionKey))
    throw new Refused(
      400,
      'unknown_region',
      `${regionKey} is not a fleet region; PRYX_REGION_MAP maps to ${options.regions.join(', ') || 'none'}`,
    )
  if (options.node !== undefined) {
    const node = await getNode(db, options.node)
    if (node === null || node.lifecycle === 'retired')
      throw new Refused(404, 'unknown_node', 'no such node, or it is retired')
    regionKey = node.region_key
  }
  const token = randomBytes(32).toString('base64url')
  const id = randomUUID()
  const row = await one<{ expires_at: string }>(
    db,
    sql`INSERT INTO fleet_node_tokens (id, token_sha256, region_key, labels, expires_at, created_by, node_id)
        VALUES (${id}, ${sha256(token)}, ${regionKey}, ${jsonb(options.labels ?? {})},
                now() + make_interval(secs => ${options.ttlSeconds}), ${options.createdBy}, ${options.node ?? null})
        RETURNING expires_at`,
  )
  await event(db, 'token.created', {
    ...(options.node ? { node: options.node } : {}),
    data: {
      id,
      region: regionKey,
      by: options.createdBy,
      reenroll: options.node !== undefined,
    },
  })
  return { id, token, expiresAt: toDate(row?.expires_at) ?? new Date() }
}

/** An address as `host:port` or `[v6]:port`, with nothing else in it. */
export const ADDRESS = /^(\[[0-9a-fA-F:.]+\]|[a-zA-Z0-9.-]+):\d{1,5}$/
export const HOST = /^(\[[0-9a-fA-F:.]+\]|[0-9a-fA-F:.]+|[a-zA-Z0-9.-]+)$/

/** One answer for unknown, spent and expired tokens: no oracle for guessing. */
const invalidToken = () => new Refused(403, 'invalid_token', 'the enrollment token is not valid')

/** What an enrolled node is told: who it is, its two certificates, and whom to trust. */
function enrolled(
  ca: FleetCa,
  options: RegistryOptions,
  nodeId: string,
  issued: { server: Issued; client: Issued },
): EnrollResponse {
  return {
    nodeId,
    deploymentId: options.deployment,
    serverCertPem: issued.server.certPem,
    clientCertPem: issued.client.certPem,
    caPem: ca.pem,
    allowedClients: [controlPlaneName(options.deployment)],
    heartbeatSeconds: options.heartbeatSeconds,
  }
}

export async function enroll(
  db: Db,
  ca: FleetCa,
  options: RegistryOptions,
  request: EnrollRequest,
): Promise<EnrollResponse> {
  if (
    typeof request?.token !== 'string' ||
    typeof request.csrPem !== 'string' ||
    typeof request.facts !== 'object'
  )
    throw new Refused(400, 'invalid_request', 'an enrollment is { token, csrPem, facts }')
  const facts = request.facts
  if (!ADDRESS.test(facts.apiAddress ?? ''))
    throw new Refused(
      400,
      'invalid_request',
      'apiAddress is host:port, where the control plane reaches pryxd',
    )
  return db.transaction(async (t) => {
    const token = await one<{
      id: string
      region_key: string
      labels: Record<string, string>
      node_id: string | null
      used_at: string | null
      used_by: string | null
      used_key_sha256: string | null
    }>(
      t,
      sql`SELECT id, region_key, labels, node_id, used_at, used_by, used_key_sha256 FROM fleet_node_tokens
          WHERE token_sha256 = ${sha256(secretOf(request.token))} AND expires_at > now() FOR UPDATE`,
    )
    if (token === null) throw invalidToken()
    if (token.used_at !== null) return enrollAgain(t, ca, options, token, request)
    if (token.node_id !== null) return reenroll(t, ca, options, token, request)
    const id = randomUUID()
    const issued = await ca
      .issueForNode(request.csrPem, id, options.deployment, options.certDays)
      .catch((error) => {
        throw new Refused(400, 'invalid_csr', (error as Error).message)
      })
    const apiHost = splitAddress(facts.apiAddress).host
    const edgeHost = facts.edgeIps.find((ip) => HOST.test(ip)) ?? apiHost
    const controlHost = facts.controlIps.find((ip) => HOST.test(ip)) ?? apiHost
    await t.execute(sql`
      INSERT INTO fleet_nodes (id, name, region_key, api_address, edge_host, control_host, cert_sha256,
        cert_expires_at, machine_id_sha256, boot_id, capacity, daemon_version, features, labels)
      VALUES (${id}, ${facts.hostname}, ${token.region_key}, ${facts.apiAddress}, ${edgeHost}, ${controlHost},
        ${issued.client.sha256}, ${issued.client.expiresAt}, ${facts.machineIdSha256}, ${facts.bootId},
        ${jsonb(facts.capacity)}, ${facts.daemonVersion}, ${jsonb(facts.features)},
        ${jsonb({ ...facts.labels, ...token.labels })})`)
    await t.execute(sql`
      UPDATE fleet_node_tokens SET used_at = now(), used_by = ${id}, used_key_sha256 = ${csrKeySha256(request.csrPem)}
      WHERE id = ${token.id}`)
    await event(t, 'node.enrolled', {
      node: id,
      data: {
        name: facts.hostname,
        region: token.region_key,
        apiAddress: facts.apiAddress,
        version: facts.daemonVersion,
      },
    })
    if (facts.machineIdSha256 !== null) {
      const twins = await rows<{ id: string }>(
        t,
        sql`SELECT id FROM fleet_nodes
            WHERE machine_id_sha256 = ${facts.machineIdSha256} AND id <> ${id} AND lifecycle <> 'retired'`,
      )
      // An image that kept its machine-id: the hosts are distinct, but the image wasn't prepared.
      if (twins.length > 0)
        await event(t, 'node.machine_id_reused', {
          node: id,
          data: { with: twins.map((r) => r.id) },
        })
    }
    return enrolled(ca, options, id, issued)
  })
}

/**
 * A spent token again, with a request for the key it enrolled, before the token's own expiry: the
 * answer never reached the node, which asks again with the same key. It is answered as it was, for
 * the node the token made, with new certificates for that key pinned in place of the ones it never
 * received. Nobody but the node can use them: they are worth nothing without its key, which never
 * left it. Any other use of a spent token is refused as an unknown one is.
 */
async function enrollAgain(
  t: Queryable,
  ca: FleetCa,
  options: RegistryOptions,
  token: { id: string; used_by: string | null; used_key_sha256: string | null },
  request: EnrollRequest,
): Promise<EnrollResponse> {
  let key: string | null
  try {
    key = csrKeySha256(request.csrPem)
  } catch {
    key = null
  }
  if (key === null || token.used_by === null || key !== token.used_key_sha256) throw invalidToken()
  const node = await getNode(t, token.used_by, true)
  if (node === null || node.lifecycle === 'retired') throw invalidToken()
  const issued = await ca
    .issueForNode(request.csrPem, node.id, options.deployment, options.certDays)
    .catch(() => {
      throw invalidToken()
    })
  await t.execute(sql`
    UPDATE fleet_nodes SET cert_sha256 = ${issued.client.sha256}, cert_expires_at = ${issued.client.expiresAt},
      next_cert_sha256 = NULL, next_cert_expires_at = NULL, updated_at = now()
    WHERE id = ${node.id}`)
  await event(t, 'node.enrollment_repeated', {
    node: node.id,
    data: {
      token: token.id,
      apiAddress: request.facts.apiAddress,
      version: request.facts.daemonVersion,
    },
  })
  return enrolled(ca, options, node.id, issued)
}

/**
 * A node enrolled again under its own id, with a token an operator bound to it: new certificates
 * pinned in place of the old, its addresses and facts refreshed, its session started afresh. Its
 * placements, lifecycle and history stay, so the host adopts its own sandboxes again. The old
 * certificates stop working at once, which also ends any copy of the host that held them.
 */
async function reenroll(
  t: Queryable,
  ca: FleetCa,
  options: RegistryOptions,
  token: { id: string; node_id: string | null },
  request: EnrollRequest,
): Promise<EnrollResponse> {
  const facts = request.facts
  const node = await getNode(t, token.node_id ?? '', true)
  if (node === null || node.lifecycle === 'retired') throw invalidToken()
  const issued = await ca
    .issueForNode(request.csrPem, node.id, options.deployment, options.certDays)
    .catch((error) => {
      throw new Refused(400, 'invalid_csr', (error as Error).message)
    })
  const apiHost = splitAddress(facts.apiAddress).host
  const edgeHost = facts.edgeIps.find((ip) => HOST.test(ip)) ?? apiHost
  const controlHost = facts.controlIps.find((ip) => HOST.test(ip)) ?? apiHost
  await t.execute(sql`
    UPDATE fleet_nodes SET cert_sha256 = ${issued.client.sha256}, cert_expires_at = ${issued.client.expiresAt},
      next_cert_sha256 = NULL, next_cert_expires_at = NULL, api_address = ${facts.apiAddress},
      edge_host = ${edgeHost}, control_host = ${controlHost}, machine_id_sha256 = ${facts.machineIdSha256},
      boot_id = ${facts.bootId}, capacity = ${jsonb(facts.capacity)}, daemon_version = ${facts.daemonVersion},
      features = ${jsonb(facts.features)}, session_id = NULL, previous_session_id = NULL,
      session_started_at = NULL, heartbeat_seq = 0, duplicate_seen_at = NULL, updated_at = now()
    WHERE id = ${node.id}`)
  await t.execute(sql`
    UPDATE fleet_node_tokens SET used_at = now(), used_by = ${node.id}, used_key_sha256 = ${csrKeySha256(request.csrPem)}
    WHERE id = ${token.id}`)
  await event(t, 'node.reenrolled', {
    node: node.id,
    data: {
      name: facts.hostname,
      apiAddress: facts.apiAddress,
      version: facts.daemonVersion,
      sameMachine: node.machine_id_sha256 === null || node.machine_id_sha256 === facts.machineIdSha256,
    },
  })
  return enrolled(ca, options, node.id, issued)
}
