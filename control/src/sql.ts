// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

import type { Queryable } from '@blockly/db'
import { type SQL, sql } from 'drizzle-orm'

/**
 * The fleet's queries are SQL, as written, through the application's database handle: the
 * statements that matter here (compare-and-set on an epoch, a heartbeat's batched upsert, a
 * region's lock) read more plainly as SQL than as a query builder.
 */

export async function rows<T>(q: Queryable, query: SQL): Promise<T[]> {
  const result = await q.execute(query)
  return result.rows as T[]
}

export async function one<T>(q: Queryable, query: SQL): Promise<T | null> {
  return (await rows<T>(q, query))[0] ?? null
}

export async function exec(q: Queryable, query: SQL): Promise<number> {
  return (await q.execute(query)).rowCount ?? 0
}

/**
 * A timestamp as these queries return it. Through the application's handle, Postgres's own text
 * for a timestamptz ("2026-10-01 12:34:56.789123+00") comes back as it is, which JavaScript's date
 * parsing doesn't promise to read; this reads it, to the millisecond.
 */
export function toDate(value: string | Date): Date
export function toDate(value: string | Date | null | undefined): Date | null
export function toDate(value: string | Date | null | undefined): Date | null {
  if (value === null || value === undefined) return null
  if (value instanceof Date) return value
  const match = /^(\d{4}-\d{2}-\d{2})[ T](\d{2}:\d{2}:\d{2})(\.\d+)?(Z|[+-]\d{2}(?::?\d{2})?)$/.exec(value)
  if (match === null) return new Date(value)
  const [, day, time, fraction = '', zone = 'Z'] = match
  const offset =
    zone === 'Z'
      ? 'Z'
      : zone.length === 3
        ? `${zone}:00`
        : zone.includes(':')
          ? zone
          : `${zone.slice(0, 3)}:${zone.slice(3)}`
  return new Date(`${day}T${time}${fraction.slice(0, 4)}${offset}`)
}

/** A JSON value as a jsonb parameter. */
export const jsonb = (value: unknown) => sql`${JSON.stringify(value)}::jsonb`

/**
 * A list as one parameter, for `= ANY(…)`: never expanded into one parameter per element, and an
 * empty list is an empty array rather than NULL, which `NOT (x = ANY(…))` would read as unknown.
 */
export const textArray = (values: readonly string[]) =>
  sql`ARRAY(SELECT jsonb_array_elements_text(${JSON.stringify(values)}::jsonb))`

/** A transaction-scoped advisory lock: held until the transaction ends. */
export async function lock(q: Queryable, key: string): Promise<void> {
  await q.execute(sql`SELECT pg_advisory_xact_lock(hashtextextended(${key}, 0))`)
}

export interface EventFields {
  node?: string | null
  workload?: string | null
  epoch?: number | null
  data?: object
}

/** What the fleet did and saw, for operators; never read back to decide anything. */
export async function event(q: Queryable, kind: string, fields: EventFields = {}): Promise<void> {
  await q.execute(sql`
    INSERT INTO fleet_events (kind, node_id, workload, epoch, data)
    VALUES (${kind}, ${fields.node ?? null}, ${fields.workload ?? null}, ${fields.epoch ?? null},
            ${jsonb(fields.data ?? {})})`)
}
