// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

import { z } from 'zod'
import type { RuntimeHandle, SnapshotHandle } from '../../app/ports/runtime.ts'

/**
 * What the fleet's handles hold. Only this adapter reads them; the application stores the string
 * and hands it back. Every new home for a world (a first placement, a move, a restore, a rebuild)
 * issues a new handle with a new epoch.
 */

const Ref = z.object({
  deployment: z.string(),
  key: z.string(),
  /** Null once released: the world rests in the archive store and the handle names no node. */
  node: z.string().nullable(),
  nodeName: z.string().nullable(),
  epoch: z.number().int().nonnegative(),
  region: z.string(),
  /**
   * Where the node was reached when the handle was issued. `endpoint()` prefers the registry's
   * current address, so a node whose address changes keeps its routes.
   */
  edgeHost: z.string().nullable(),
  controlHost: z.string().nullable(),
  /** Port name → host port on the node. */
  ports: z.record(z.string(), z.number().int()),
})
export type FleetRef = z.infer<typeof Ref>

const Snap = z.object({
  deployment: z.string(),
  key: z.string(),
  /** The `fleet_archives` row: its node copy, and its copy in the archive store once uploaded. */
  archive: z.string(),
  /** The placement epoch the copy was taken at. */
  epoch: z.number().int().nonnegative(),
})
export type SnapshotRef = z.infer<typeof Snap>

const PREFIX = 'fleet:v1:'
const SNAPSHOT_PREFIX = 'fleet-snap:v1:'

const pack = (prefix: string, value: object) =>
  `${prefix}${Buffer.from(JSON.stringify(value)).toString('base64url')}`

function unpack<T>(prefix: string, schema: z.ZodType<T>, text: string, what: string): T {
  if (!text.startsWith(prefix)) throw new Error(`This ${what} was not issued by the fleet runtime`)
  let json: unknown = null
  try {
    json = JSON.parse(Buffer.from(text.slice(prefix.length), 'base64url').toString())
  } catch {
    json = null
  }
  const parsed = schema.safeParse(json)
  if (!parsed.success) throw new Error(`Malformed fleet ${what}`)
  return parsed.data
}

export const encodeHandle = (ref: FleetRef) => pack(PREFIX, Ref.parse(ref)) as RuntimeHandle
export const decodeHandle = (handle: string): FleetRef => unpack(PREFIX, Ref, handle, 'handle')
export const isHandle = (value: string) => value.startsWith(PREFIX)

export const encodeSnapshot = (ref: SnapshotRef) => pack(SNAPSHOT_PREFIX, Snap.parse(ref)) as SnapshotHandle
export const decodeSnapshot = (handle: string): SnapshotRef =>
  unpack(SNAPSHOT_PREFIX, Snap, handle, 'snapshot')
export const isSnapshot = (value: string) => value.startsWith(SNAPSHOT_PREFIX)
