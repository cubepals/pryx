// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Stable public addresses and raw port reservations; the database serializes allocation and collisions. */

import { sql } from 'drizzle-orm'
import type { PublicPort } from './api-schemas.ts'
import type { Queryable } from './database.ts'
import { MINUTE_S } from './durations.ts'

import { HTTP_FRONT_PORT, TLS_FRONT_PORT } from './edge-headers.ts'
import { SandboxError } from './sandboxes.ts'
import { lock, one, rows } from './sql.ts'
export const DEFAULT_EDGE_PORT_RANGE = [20000, 20100] as const
export interface PublicAddress {
  name: string
  hostname: string
  front: 'http' | 'tls' | 'port'
  public_port: number
}
export async function addresses(
  db: Queryable,
  id: string,
  ports: PublicPort[],
  domain: string,
  range: readonly [number, number] = DEFAULT_EDGE_PORT_RANGE,
) {
  return db.transaction(async (t) => {
    await lock(t, 'edge-addresses')
    // Public port reuse waits ten minutes, matching the node's default quarantine.
    await t.execute(
      sql`DELETE FROM edge_addresses WHERE retired_at < now()-(${10 * MINUTE_S}*interval '1 second')`,
    )
    for (const p of ports.filter((p) => p.audience === 'public')) {
      if (p.protocol !== 'tcp')
        throw new SandboxError('invalid_request', 'Public edge fronts support TCP; UDP ports must be private')
      if (await one(t, sql`SELECT 1 FROM edge_addresses WHERE sandbox=${id} AND name=${p.name}`)) continue
      const front = p.front ?? (p.tls ? 'tls' : 'http'),
        hostname = `${p.alias ?? id}.${domain}`.toLowerCase()
      let port = front === 'http' ? HTTP_FRONT_PORT : p.tls ? p.port : TLS_FRONT_PORT
      if (front === 'port') {
        const chosen = await one<{ port: number }>(
          t,
          sql`SELECT p AS port FROM generate_series(${range[0]}::int,${range[1]}::int) p WHERE NOT EXISTS (SELECT 1 FROM edge_addresses WHERE public_port=p) ORDER BY p LIMIT 1`,
        )
        if (!chosen) throw new SandboxError('no_room', 'Edge public port range is exhausted')
        port = chosen.port
      }
      const collision = await one(
        t,
        sql`SELECT 1 FROM edge_addresses WHERE (hostname=${hostname} AND front=${front} AND public_port=${port}) OR (public_port=${port} AND (front='port' OR ${front}='port'))`,
      )
      if (collision)
        throw new SandboxError(
          'invalid_request',
          'Public hostname or port is already assigned; use a distinct alias for each port',
        )
      await t.execute(
        sql`INSERT INTO edge_addresses(sandbox,name,hostname,front,public_port) VALUES(${id},${p.name},${hostname},${front},${port})`,
      )
    }
    return rows<PublicAddress>(
      t,
      sql`SELECT name,hostname,front,public_port FROM edge_addresses WHERE sandbox=${id} AND retired_at IS NULL`,
    )
  })
}
