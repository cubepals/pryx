// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/**
 * What the fleet runtime's synchronous methods know of each node (its address, lifecycle, region,
 * hosts and price), read from `fleet_nodes` now and every few seconds after. A read that fails
 * keeps the last one and is logged: a node's routes go on to an address a few seconds old.
 *
 * It decides nothing about nodes and writes no row: the registry (`../registry.ts`) owns them.
 */

import { sql } from 'drizzle-orm'

import type { Queryable } from './database.ts'
import { SECOND_MS } from './durations.ts'
import type { Lifecycle } from './placement.ts'
import { rows } from './sql.ts'

/** How often each process rereads node addresses and lifecycles, which pure methods read. */
const REGISTRY_REFRESH_MS = 5 * SECOND_MS

/** What pure methods know of a node, refreshed every few seconds. */
export interface CachedNode {
  apiAddress: string
  lifecycle: Lifecycle
  region: string
  edgeHost: string
  controlHost: string
  /** A node's monthly cost, in US cents, when its operator gave one (label `monthly_cost_cents`). */
  monthlyCents: number | null
  allocatableMemoryMb: number
}

export class NodeCache {
  readonly #db: Queryable
  readonly #log: (message: string, fields?: object) => void
  #nodes = new Map<string, CachedNode>()
  #timer: NodeJS.Timeout | null = null

  constructor(db: Queryable, log: (message: string, fields?: object) => void) {
    this.#db = db
    this.#log = log
  }

  get(id: string): CachedNode | undefined {
    return this.#nodes.get(id)
  }

  /** Reads the registry now and every few seconds after, until the returned function is called. */
  async watch(): Promise<() => void> {
    await this.refresh()
    this.#timer = setInterval(() => {
      this.refresh().catch((error) =>
        this.#log('registry refresh failed', {
          error: (error as Error).message,
        }),
      )
    }, REGISTRY_REFRESH_MS)
    this.#timer.unref()
    return () => {
      if (this.#timer) clearInterval(this.#timer)
      this.#timer = null
    }
  }

  async refresh(): Promise<void> {
    const found = await rows<{
      id: string
      api_address: string
      lifecycle: Lifecycle
      region_key: string
      edge_host: string
      control_host: string
      labels: Record<string, string>
      capacity: { allocatableMemoryMb?: number }
    }>(
      this.#db,
      sql`SELECT id, api_address, lifecycle, region_key, edge_host, control_host, labels, capacity FROM fleet_nodes`,
    )
    this.#nodes = new Map(
      found.map((r) => {
        const cents = Number(r.labels.monthly_cost_cents)
        return [
          r.id,
          {
            apiAddress: r.api_address,
            lifecycle: r.lifecycle,
            region: r.region_key,
            edgeHost: r.edge_host,
            controlHost: r.control_host,
            monthlyCents: Number.isFinite(cents) && cents > 0 ? cents : null,
            allocatableMemoryMb: r.capacity.allocatableMemoryMb ?? 0,
          },
        ]
      }),
    )
  }
}
