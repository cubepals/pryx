// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/** The timestamp columns every table writes the same way: with a time zone, set on insert. */
import { timestamp } from 'drizzle-orm/pg-core'

export const ts = (name: string) => timestamp(name, { withTimezone: true })
export const createdAt = () => ts('created_at').notNull().defaultNow()
export const updatedAt = () => ts('updated_at').notNull().defaultNow()
