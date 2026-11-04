// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/**
 * Renewing an enrolled node's certificates, asked for over mutual TLS with its current one.
 * The new certificate takes over when a heartbeat presents it, in `heartbeat.ts`; a machine's first
 * certificates come from `enrollment.ts`.
 */
import type { Db } from '@blockly/db'
import { sql } from 'drizzle-orm'
import type { FleetCa } from '../ca.ts'
import { event } from '../sql.ts'
import type { RenewRequest, RenewResponse } from '../wire.ts'
import { certificateOf } from './heartbeat.ts'
import { getNode, Refused, type RegistryOptions } from './node.ts'

// ─── renewal ────────────────────────────────────────────────────────────────────────────────

/**
 * New certificates for a key the node made just now, asked for over mutual TLS with its current
 * certificate. The new client certificate works alongside the current one until a heartbeat
 * presents it; a renewal the node never received changes nothing for it.
 */
export async function renew(
  db: Db,
  ca: FleetCa,
  options: RegistryOptions,
  nodeId: string,
  peerCertSha256: string,
  request: RenewRequest,
): Promise<RenewResponse> {
  if (request?.nodeId !== nodeId || typeof request.csrPem !== 'string')
    throw new Refused(400, 'invalid_request', 'a renewal is { nodeId, csrPem }, for the node that asks')
  return db.transaction(async (t) => {
    const node = await getNode(t, nodeId, true)
    if (node === null) throw new Refused(404, 'unknown_node', 'no such node')
    if (node.lifecycle === 'retired' || node.lifecycle === 'lost')
      throw new Refused(403, node.lifecycle, `a ${node.lifecycle} node's certificates are not renewed`)
    if (node.duplicate_seen_at !== null)
      throw new Refused(403, 'quarantined', 'two hosts beat with this identity')
    if (certificateOf(node, peerCertSha256) !== 'current')
      throw new Refused(403, 'certificate_not_current', "only the node's current certificate may renew")
    const issued = await ca
      .issueForNode(request.csrPem, nodeId, options.deployment, options.certDays)
      .catch((error) => {
        throw new Refused(400, 'invalid_csr', (error as Error).message)
      })
    await t.execute(sql`
      UPDATE fleet_nodes SET next_cert_sha256 = ${issued.client.sha256},
        next_cert_expires_at = ${issued.client.expiresAt}, updated_at = now() WHERE id = ${nodeId}`)
    await event(t, 'node.certificate_issued', { node: nodeId, data: { expiresAt: issued.client.expiresAt } })
    return { serverCertPem: issued.server.certPem, clientCertPem: issued.client.certPem, caPem: ca.pem }
  })
}
