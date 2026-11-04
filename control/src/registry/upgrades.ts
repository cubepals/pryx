// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/**
 * The pryxd rollout (docs/fleet.md, "Upgrades"): the control plane offers its own pryxd to
 * nodes running an older one, one node per region at a time, in the answer to a heartbeat.
 *
 * - A node is offered the release only while it is active, not quarantined, beats healthy and
 *   lists `self-upgrade`, and only while no other node in its region holds an offer of it or
 *   failed it.
 * - It is `upgraded` once it beats healthy on the release, which frees its region for the next.
 * - It `failed` when it reports giving up (pryxd rolled itself back), or when it isn't healthy
 *   on the release within UPGRADE_TIMEOUT_SECONDS of the offer. A failure stops its region's
 *   rollout of that release until an operator retries the node (`retryUpgrade`).
 *
 * The state is a node's own (`fleet_nodes.upgrade_*`), written only in that node's heartbeat
 * transaction, which already holds its row: a beat never writes another node's row, so a node
 * that went quiet mid-upgrade keeps its region waiting until it beats again. The release is not
 * state: it is the binary this process serves, so a new control-plane image starts a new rollout.
 * Which nodes are offered what is read back by `upgradeSummaries`, for `fleet.ts upgrades`.
 */

import { sql } from 'drizzle-orm'
import type { Queryable } from '../database.ts'

import { MINUTE_S } from '../durations.ts'
import { event, exec, lock, one, rows } from '../sql.ts'
import type { HeartbeatRequest, UpgradeOffer } from '../wire.ts'
import { type NodeRow, Refused } from './node.ts'

/** How long an offered node has to download, restart, and beat healthy on the new version. */
export const UPGRADE_TIMEOUT_SECONDS = 900

type Version = [number, number, number]

function parse(version: string | null | undefined): Version | null {
  const match = /^(\d+)\.(\d+)\.(\d+)/.exec(version ?? '')
  return match ? [Number(match[1]), Number(match[2]), Number(match[3])] : null
}

/** Whether `version` is older than `than`. A version that isn't `x.y.z` is never older. */
export function isOlder(version: string | null | undefined, than: string): boolean {
  const [a, b] = [parse(version), parse(than)]
  if (a === null || b === null) return false
  for (let i = 0; i < 3; i++) if ((a[i] ?? 0) !== (b[i] ?? 0)) return (a[i] ?? 0) < (b[i] ?? 0)
  return false
}

/** The feature a node lists when it upgrades itself on an offer; one without is upgraded by hand. */
const SELF_UPGRADE = 'self-upgrade'
/**
 * The feature a node lists when it installs only a pryxd whose release signed it: an offer to it
 * carries the release's signature, and without one it isn't made (pryxd's src/upgrade/trust.rs).
 */
const SIGNED_UPGRADE = 'signed-upgrade'

/** Why a node is never offered `release` as it is now, if it isn't. */
function skipped(
  node: Pick<NodeRow, 'lifecycle' | 'duplicate_seen_at' | 'features'>,
  release: UpgradeOffer,
): string | null {
  if (node.duplicate_seen_at !== null) return 'quarantined'
  if (node.lifecycle !== 'active') return node.lifecycle
  if (!node.features.includes(SELF_UPGRADE)) return 'it upgrades by hand: too old to upgrade itself'
  if (node.features.includes(SIGNED_UPGRADE) && !release.signature)
    return 'it takes only a signed pryxd, and this one has no pryxd.sig beside it'
  return null
}

/** `release` as a node that lists `features` is offered it: with the signature only if it reads one. */
function offering(release: UpgradeOffer, features: readonly string[]): UpgradeOffer {
  const { signature, ...offer } = release
  return features.includes(SIGNED_UPGRADE) && signature ? { ...offer, signature } : offer
}

async function mark(
  t: Queryable,
  node: NodeRow,
  state: 'upgraded' | 'failed',
  data: { version: string; reason?: string },
): Promise<void> {
  await t.execute(sql`
    UPDATE fleet_nodes SET upgrade_state = ${state}, upgrade_at = now(), upgrade_error = ${data.reason ?? null}
    WHERE id = ${node.id}`)
  await event(t, state === 'upgraded' ? 'node.upgraded' : 'node.upgrade_failed', {
    node: node.id,
    data,
  })
}

/**
 * What one heartbeat does to the rollout, in its transaction: `node` as it was before the beat,
 * `request` the beat. Returns the offer to answer with, if any.
 */
export async function rollout(
  t: Queryable,
  node: NodeRow,
  request: HeartbeatRequest,
  release: UpgradeOffer | null | undefined,
): Promise<UpgradeOffer | undefined> {
  const offered = node.upgrade_state === 'offered'
  const failure = request.upgradeFailed
  if (offered && failure && failure.version === node.upgrade_version) {
    await mark(t, node, 'failed', {
      version: failure.version,
      reason: String(failure.reason).slice(0, 500),
    })
    return undefined
  }
  if (!release) return undefined
  if (offered && node.upgrade_version === release.version) return follow(t, node, request, release)
  return offer(t, node, request, release)
}

/** A node that holds an offer of `release`: upgraded, failed, still on its way, or withdrawn. */
async function follow(
  t: Queryable,
  node: NodeRow,
  request: HeartbeatRequest,
  release: UpgradeOffer,
): Promise<UpgradeOffer | undefined> {
  const late = (node.upgrade_age ?? 0) > UPGRADE_TIMEOUT_SECONDS
  const minutes = UPGRADE_TIMEOUT_SECONDS / MINUTE_S
  const { version } = release
  if (!isOlder(request.daemonVersion, version)) {
    if (request.runtimeUp && request.reconciled) await mark(t, node, 'upgraded', { version })
    else if (late)
      await mark(t, node, 'failed', {
        version,
        reason: `it runs ${version}, but didn't beat healthy within ${minutes} minutes`,
      })
    return undefined
  }
  if (late) {
    const reason = `it still runs ${request.daemonVersion} ${minutes} minutes after the offer`
    await mark(t, node, 'failed', { version, reason })
    return undefined
  }
  if (node.lifecycle === 'active' && node.duplicate_seen_at === null)
    return offering(release, request.features)
  // Drained, lost or quarantined while offered: the offer is withdrawn and the region moves on.
  await t.execute(sql`UPDATE fleet_nodes SET upgrade_state = NULL, upgrade_at = now() WHERE id = ${node.id}`)
  return undefined
}

/** A node that holds no offer of `release`: offered it, when it is older and its turn has come. */
async function offer(
  t: Queryable,
  node: NodeRow,
  request: HeartbeatRequest,
  release: UpgradeOffer,
): Promise<UpgradeOffer | undefined> {
  if (!isOlder(request.daemonVersion, release.version)) return undefined
  if (node.upgrade_version === release.version && node.upgrade_state === 'failed') return undefined
  if (skipped({ ...node, features: request.features }, release) !== null) return undefined
  if (!request.runtimeUp || !request.reconciled) return undefined

  // One node per region: two beats from one region decide one after the other.
  await lock(t, `fleet:upgrade:${node.region_key}`)
  const busy = await one<{ id: string }>(
    t,
    sql`SELECT id FROM fleet_nodes
        WHERE region_key = ${node.region_key} AND id <> ${node.id} AND lifecycle <> 'retired'
          AND upgrade_version = ${release.version} AND upgrade_state IN ('offered', 'failed')
        LIMIT 1`,
  )
  if (busy !== null) return undefined
  await t.execute(sql`
    UPDATE fleet_nodes SET upgrade_version = ${release.version}, upgrade_state = 'offered', upgrade_at = now(),
      upgrade_error = NULL
    WHERE id = ${node.id}`)
  await event(t, 'node.upgrade_offered', {
    node: node.id,
    data: { from: request.daemonVersion, version: release.version },
  })
  return offering(release, request.features)
}

/** A failed node offered the release again, which resumes its region's rollout. */
export async function retryUpgrade(q: Queryable, nodeId: string, by: string): Promise<void> {
  const cleared = await exec(
    q,
    sql`UPDATE fleet_nodes SET upgrade_state = NULL, upgrade_at = now(), upgrade_error = NULL
        WHERE id = ${nodeId} AND upgrade_state = 'failed'`,
  )
  if (cleared === 0) throw new Refused(409, 'not_failed', 'this node has no failed upgrade to retry')
  await event(q, 'node.upgrade_retried', { node: nodeId, data: { by } })
}

export interface UpgradeView {
  id: string
  name: string
  region: string
  lifecycle: string
  version: string | null
  /**
   * `current`: it runs the release or newer (`upgraded` if the rollout got it there). `offered`
   * and `failed`, as above. `waiting`: older, and its turn hasn't come. `skipped`: older, and
   * not offered while it is draining, lost, quarantined, too old to upgrade itself, or takes only
   * a signed pryxd and this one has no signature (`reason`).
   */
  state: 'current' | 'upgraded' | 'offered' | 'failed' | 'waiting' | 'skipped'
  since: string | null
  reason: string | null
}

/** Every node that isn't retired, and where it is in the rollout of `release`. */
export async function upgradeSummaries(q: Queryable, release: UpgradeOffer | null): Promise<UpgradeView[]> {
  const nodes = await rows<NodeRow>(
    q,
    sql`SELECT n.*, EXTRACT(EPOCH FROM now() - n.upgrade_at)::float8 AS upgrade_age FROM fleet_nodes n
        WHERE n.lifecycle <> 'retired' ORDER BY n.region_key, n.name, n.id`,
  )
  return nodes.map((n) => ({
    id: n.id,
    name: n.name,
    region: n.region_key,
    lifecycle: n.lifecycle,
    version: n.daemon_version,
    since: n.upgrade_at === null ? null : String(n.upgrade_at),
    ...placeOf(n, release),
  }))
}

/** Where one node is in the rollout of `release`, as `UpgradeView` says. */
function placeOf(n: NodeRow, release: UpgradeOffer | null): Pick<UpgradeView, 'state' | 'reason'> {
  const ours = release !== null && n.upgrade_version === release.version
  if (release === null || !isOlder(n.daemon_version, release.version))
    return {
      state: ours && n.upgrade_state === 'upgraded' ? 'upgraded' : 'current',
      reason: null,
    }
  if (ours && n.upgrade_state === 'failed') return { state: 'failed', reason: n.upgrade_error }
  if (ours && n.upgrade_state === 'offered') {
    const late = (n.upgrade_age ?? 0) > UPGRADE_TIMEOUT_SECONDS
    return {
      state: 'offered',
      reason: late ? 'past its time: it fails at its next beat' : null,
    }
  }
  const skip = skipped(n, release)
  return { state: skip === null ? 'waiting' : 'skipped', reason: skip }
}
