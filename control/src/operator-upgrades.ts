// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

import type { Hono } from 'hono'
import type { Db } from './database.ts'
import { OPERATOR_BASE } from './operator-api.ts'
import { Refused, retryUpgrade, UPGRADE_TIMEOUT_SECONDS, upgradeSummaries } from './registry.ts'
import type { UpgradeOffer } from './wire.ts'

/**
 * The operators' view of the pryxd rollout (registry/upgrades.ts), added to the operator API
 * `createOperatorApi` made, behind its token: `fleet.ts upgrades` and `fleet.ts retry-upgrade`.
 *
 * - `GET /fleet/v1/upgrades`: the release this process rolls out, and each node's version and
 *   place in the rollout.
 * - `POST /fleet/v1/nodes/:id/retry-upgrade`: a node whose upgrade failed is offered it again,
 *   which resumes its region's rollout.
 */
export function withUpgrades(app: Hono, options: { db: Db; release: UpgradeOffer | null }): Hono {
  app.get(`${OPERATOR_BASE}/upgrades`, async (c) =>
    c.json({
      release: options.release,
      timeoutSeconds: UPGRADE_TIMEOUT_SECONDS,
      nodes: await upgradeSummaries(options.db, options.release),
    }),
  )
  app.post(`${OPERATOR_BASE}/nodes/:id/retry-upgrade`, async (c) => {
    const id = c.req.param('id')
    if (!/^[0-9a-f-]{36}$/.test(id)) throw new Refused(400, 'invalid_node', 'a node is named by its id')
    await retryUpgrade(options.db, id, `operator:${(c.req.header('x-operator') ?? 'unnamed').slice(0, 64)}`)
    return c.json({ retried: true })
  })
  return app
}
