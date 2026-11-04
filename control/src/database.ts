// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Postgres handles and migration location; queries belong to the module owning each table. */
import { fileURLToPath } from 'node:url'
import { drizzle, type NodePgDatabase } from 'drizzle-orm/node-postgres'
import pg from 'pg'
import { SECOND_MS } from './durations.ts'
import * as schema from './schema/fleet.ts'

export type Db = NodePgDatabase<typeof schema>
export type Tx = Parameters<Parameters<Db['transaction']>[0]>[0]
export type Queryable = Db | Tx
export const migrationsFolder = fileURLToPath(new URL('./schema/migrations', import.meta.url))
// PostgreSQL SQLSTATE, lock_not_available: https://www.postgresql.org/docs/current/errcodes-appendix.html
export const LOCK_NOT_AVAILABLE = '55P03'
export const createPool = (connectionString: string, max = 10) => {
  const pool = new pg.Pool({
    connectionString,
    max,
    idleTimeoutMillis: 70 * SECOND_MS,
  })
  pool.on('error', (error) => console.error('postgres: resting connection closed', error.message))
  return pool
}
export const createDb = (pool: pg.Pool): Db => drizzle(pool, { schema })
