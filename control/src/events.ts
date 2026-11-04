// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

import { sql } from 'drizzle-orm'
import type { Db } from './database.ts'
import { rows } from './sql.ts'

export const events = (db: Db, sandbox: string, since?: string) =>
  rows(
    db,
    sql`
  SELECT id, at, kind, epoch, data FROM fleet_events WHERE workload = ${sandbox}
  ${since ? sql`AND at > ${since}::timestamptz` : sql``} ORDER BY id LIMIT 1000`,
  )
