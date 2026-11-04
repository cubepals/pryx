// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/**
 * The fleet runtime's own state (docs/fleet.md): its nodes and their tokens, archives on them,
 * placements and their history, what the nodes observe, their endpoints and events.
 */

import {
  bigint,
  boolean,
  index,
  integer,
  jsonb,
  pgEnum,
  pgTable,
  primaryKey,
  text,
  uuid,
} from 'drizzle-orm/pg-core'
import { createdAt, ts, updatedAt } from './columns.ts'

// ─── The fleet runtime's own state (docs/fleet.md) ──────────────────────────────────────────
// Read and written only by infra/fleet: the application sees a fleet server through its handle,
// as it sees a Fly machine. Every time here is the database's; no clock is compared across
// machines. A node is never identified by its address.

/**
 * What an operator decided about a node. `draining` takes no new sandboxes; `lost` is an
 * operator's statement that the host is gone, which lets its sandboxes be rebuilt elsewhere;
 * `retired` is gone for good and its certificate no longer works. Health (healthy, suspect,
 * unavailable…) is derived from heartbeats, never stored as a decision.
 */
export const fleetNodeLifecycle = pgEnum('fleet_node_lifecycle', ['active', 'draining', 'lost', 'retired'])

/**
 * Where a server's data lives. `placing`: a node is chosen and the copy may be under way;
 * `placed`: the copy exists there; `released`: compute and storage were let go and the data
 * rests in the archive store; `displaced`: its node was declared lost and it waits to be rebuilt.
 */
export const fleetPlacementState = pgEnum('fleet_placement_state', [
  'placing',
  'placed',
  'released',
  'displaced',
])

export const fleetNodes = pgTable(
  'fleet_nodes',
  {
    /** Issued at enrollment; the name in the node's certificates is derived from it. */
    id: uuid('id').primaryKey(),
    /** The host's name when it enrolled, for people. */
    name: text('name').notNull(),
    /**
     * The fleet region it enrolled into, which product regions map to and placement compares (the
     * token said so, not the node).
     */
    regionKey: text('region_key').notNull(),
    /** Where the control plane dials pryxd's API, host:port; reported by the node, may change. */
    apiAddress: text('api_address').notNull(),
    /** Where the edge reaches its sandboxes' public ports. */
    edgeHost: text('edge_host').notNull(),
    /** Where the control plane reaches their private ports. */
    controlHost: text('control_host').notNull(),
    lifecycle: fleetNodeLifecycle('lifecycle').notNull().default('active'),
    lostAt: ts('lost_at'),
    lostReason: text('lost_reason'),
    /** How the operator says the lost host was stopped: power-off, provider, network, none. */
    fencedBy: text('fenced_by'),
    /**
     * The sha256 of the node's current client certificate. Heartbeats with any other are refused,
     * so renewal replaces it and revocation clears it. A lost node keeps it, so its return is known.
     */
    certSha256: text('cert_sha256'),
    certExpiresAt: ts('cert_expires_at'),
    /**
     * A renewed client certificate, issued but not yet seen: both work until a heartbeat comes
     * with this one, which then becomes the current one. A renewal the node never received
     * doesn't lock it out.
     */
    nextCertSha256: text('next_cert_sha256'),
    nextCertExpiresAt: ts('next_cert_expires_at'),
    /** sha256 of /etc/machine-id: two nodes sharing one means an image that wasn't prepared. */
    machineIdSha256: text('machine_id_sha256'),
    /** The kernel's boot id at the last heartbeat; a new one is a reboot. */
    bootId: text('boot_id'),
    /** The daemon process now beating; a new one is a restart. */
    sessionId: text('session_id'),
    sessionStartedAt: ts('session_started_at'),
    previousSessionId: text('previous_session_id'),
    /** Set when two hosts beat with one identity: no new sandboxes until an operator clears it. */
    duplicateSeenAt: ts('duplicate_seen_at'),
    /** The current session's last sequence number: an older or repeated beat changes nothing. */
    heartbeatSeq: bigint('heartbeat_seq', { mode: 'number' }).notNull().default(0),
    lastHeartbeatAt: ts('last_heartbeat_at'),
    /** Whether the node could reach its container runtime at the last beat. */
    runtimeUp: boolean('runtime_up').notNull().default(false),
    /** Whether the node had reconciled its workloads with the runtime since it started. */
    reconciled: boolean('reconciled').notNull().default(false),
    /** The last health recorded, so a change is written once, with an event. */
    health: text('health').notNull().default('unavailable'),
    healthSince: ts('health_since').notNull().defaultNow(),
    /** What the node reported it can hold: memory, CPUs, disk, ports, and what it already uses. */
    capacity: jsonb('capacity').$type<Record<string, unknown>>().notNull().default({}),
    daemonVersion: text('daemon_version'),
    features: jsonb('features').$type<string[]>().notNull().default([]),
    /** Problems the node reported about itself (a full disk, a runtime that won't answer). */
    issues: jsonb('issues').$type<unknown[]>().notNull().default([]),
    labels: jsonb('labels').$type<Record<string, string>>().notNull().default({}),
    /** What the last probe of a silent node found: refused, timed out, or answering. */
    lastProbe: jsonb('last_probe').$type<Record<string, unknown>>(),
    /**
     * The pryxd version the upgrade rollout last offered this node (docs/fleet.md, "Upgrades"):
     * the version the control plane's own binary is, at the time.
     */
    upgradeVersion: text('upgrade_version'),
    /**
     * `offered`: the node was offered `upgradeVersion` and nobody else in its region is until it
     * beats healthy on it (`upgraded`) or doesn't (`failed`, which stops the region's rollout).
     */
    upgradeState: text('upgrade_state'),
    upgradeAt: ts('upgrade_at'),
    /** Why the upgrade failed: what the node reported, or that it never came back on the version. */
    upgradeError: text('upgrade_error'),
    enrolledAt: ts('enrolled_at').notNull().defaultNow(),
    createdAt: createdAt(),
    updatedAt: updatedAt(),
  },
  (t) => [index('fleet_nodes_region').on(t.regionKey)],
)

/** One-time enrollment tokens. Only a hash is kept; the token is shown once, when made. */
export const fleetNodeTokens = pgTable('fleet_node_tokens', {
  id: uuid('id').primaryKey(),
  tokenSha256: text('token_sha256').notNull().unique(),
  regionKey: text('region_key').notNull(),
  labels: jsonb('labels').$type<Record<string, string>>().notNull().default({}),
  expiresAt: ts('expires_at').notNull(),
  usedAt: ts('used_at'),
  usedBy: uuid('used_by').references(() => fleetNodes.id),
  /**
   * sha256 of the key the token enrolled (its SubjectPublicKeyInfo, DER). Until the token expires,
   * the node it enrolled may ask again with that key, when the answer never reached it.
   */
  usedKeySha256: text('used_key_sha256'),
  /**
   * A re-enrollment: the node this token gives new certificates to, keeping its id, so the host
   * keeps its sandboxes. For a node whose certificates ended, or whose identity directory was lost.
   */
  nodeId: uuid('node_id').references(() => fleetNodes.id),
  /** Who made it: `operator:<name>`, the name the operators' API was given (`unnamed` without one). */
  createdBy: text('created_by').notNull(),
  createdAt: createdAt(),
})

/**
 * Copies of a data the fleet keeps for itself. A snapshot is first a copy on its node
 * (`localId`), made while saving is paused, then uploaded to the archive store (`objectKey`),
 * so it outlives its node. A move's copy goes straight to the store.
 */
export const fleetArchives = pgTable(
  'fleet_archives',
  {
    id: uuid('id').primaryKey(),
    /** The runtime key: the server id. */
    workload: text('workload').notNull(),
    /** `snapshot`: a backup the application asked for. `move`: what a move carries. */
    purpose: text('purpose').notNull(),
    /** The placement epoch the copy was taken at. */
    epoch: bigint('epoch', { mode: 'number' }).notNull(),
    /** The node that made the copy. */
    nodeId: uuid('node_id').references(() => fleetNodes.id),
    /** The node's own id for its local copy; null for a move, which has none. */
    localId: text('local_id'),
    /** When the local copy was deleted, by the fleet or with its workload. */
    localDeletedAt: ts('local_deleted_at'),
    /** The object in the archive store, once an upload began. */
    objectKey: text('object_key').unique(),
    /**
     * `creating`: its node is making it; `local`: only on its node; `uploading`; `ready`: in the
     * store; `failed`: one its node couldn't make, or an upload that failed for good; `deleted`.
     */
    status: text('status').notNull(),
    /** Of the object in the store, when it is ready. */
    sha256: text('sha256'),
    sizeBytes: bigint('size_bytes', { mode: 'number' }),
    /** `stopped`: taken from a stopped server. `quiesced`: running, with saving paused by the caller. */
    consistency: text('consistency').notNull(),
    capturedAt: ts('captured_at').notNull(),
    /** When the upload under way began: one not heard from in hours is taken over. */
    uploadStartedAt: ts('upload_started_at'),
    uploadedAt: ts('uploaded_at'),
    /** Upload attempts, and the last one's error. */
    attempts: integer('attempts').notNull().default(0),
    error: text('error'),
    deletedAt: ts('deleted_at'),
    createdAt: createdAt(),
  },
  (t) => [index('fleet_archives_workload').on(t.workload, t.createdAt)],
)

/** Where each server's data is meant to live: one row per server, the desired placement. */
export const fleetPlacements = pgTable(
  'fleet_placements',
  {
    /** The runtime key: the server id. */
    workload: text('workload').primaryKey(),
    nodeId: uuid('node_id').references(() => fleetNodes.id),
    /**
     * The placement epoch: raised, in the transaction that changes the node, every time the data
     * gets a new home. Never reused and never lowered. Every command to a node carries it, and a
     * node refuses older ones, so a copy left behind can't be started.
     */
    epoch: bigint('epoch', { mode: 'number' }).notNull(),
    state: fleetPlacementState('state').notNull(),
    regionKey: text('region_key').notNull(),
    /** The reservation placement counts: what was promised, not what the node last reported. */
    memoryMb: integer('memory_mb').notNull(),
    cpuMillis: integer('cpu_millis').notNull(),
    diskGb: integer('disk_gb').notNull(),
    /** Host ports the node gave the copy, by port name. */
    ports: jsonb('ports').$type<Record<string, number>>().notNull().default({}),
    /** The handle issued for this placement. */
    handle: text('handle'),
    /** The node's digest of the spec its copy runs: what an apply's `If-Match` names. */
    specDigest: text('spec_digest'),
    /**
     * Blockly's digest of the spec last sent to this placement's copy, secrets included: a start
     * with the same spec sends none. Cleared with each new epoch.
     */
    sentDigest: text('sent_digest'),
    /**
     * What a `placing` row waits on before it is placed: null for a new placement, else the
     * restore, move or recovery that fills the copy first, which a retry resumes.
     */
    completing: text('completing'),
    /** The copy a move or recovery fills the new home from. */
    restoreFrom: uuid('restore_from').references(() => fleetArchives.id),
    /**
     * The last power command the runtime was given: a record, not a second power writer. The
     * application's server status is the only intent anything acts on.
     */
    desiredPower: text('desired_power').notNull().default('stopped'),
    /**
     * When `desiredPower` was last written. Memory is held only for sandboxes that run: a start
     * claims it, and the node reporting the copy stopped after this time gives it back
     * (docs/fleet.md, "Admission").
     */
    powerChangedAt: ts('power_changed_at').notNull().defaultNow(),
    /**
     * An operator asked for this server to move, to `moveTo` if they named a node: its next
     * relocation leaves its node even when that node could keep it. Cleared once it has moved.
     */
    moveRequestedAt: ts('move_requested_at'),
    moveTo: uuid('move_to').references(() => fleetNodes.id),
    createdAt: createdAt(),
    updatedAt: updatedAt(),
  },
  (t) => [index('fleet_placements_node').on(t.nodeId)],
)

/** Every epoch a server ever had: where, why it began, and why it ended. */
export const fleetPlacementHistory = pgTable(
  'fleet_placement_history',
  {
    workload: text('workload').notNull(),
    epoch: bigint('epoch', { mode: 'number' }).notNull(),
    nodeId: uuid('node_id').references(() => fleetNodes.id),
    /** placed, restore, move, recover. */
    reason: text('reason').notNull(),
    startedAt: ts('started_at').notNull().defaultNow(),
    endedAt: ts('ended_at'),
    /** moved, restored, released, lost, destroyed, abandoned (never made), or a node's refusal. */
    endReason: text('end_reason'),
  },
  (t) => [primaryKey({ columns: [t.workload, t.epoch] })],
)

/**
 * What each node last reported about each copy it holds: the observed state. Keyed by node,
 * since after a move two nodes may report one server, which is how a copy left behind is found.
 */
export const fleetObservations = pgTable(
  'fleet_observations',
  {
    nodeId: uuid('node_id')
      .notNull()
      .references(() => fleetNodes.id),
    workload: text('workload').notNull(),
    epoch: bigint('epoch', { mode: 'number' }),
    /** The epoch that fenced this copy, if one did. */
    supersededBy: bigint('superseded_by', { mode: 'number' }),
    state: text('state').notNull(),
    specDigest: text('spec_digest'),
    report: jsonb('report').$type<Record<string, unknown>>().notNull(),
    /** When this report last changed; the node's last heartbeat says how fresh it is. */
    observedAt: ts('observed_at').notNull().defaultNow(),
    stateChangedAt: ts('state_changed_at').notNull().defaultNow(),
  },
  (t) => [
    primaryKey({ columns: [t.nodeId, t.workload] }),
    index('fleet_observations_workload').on(t.workload),
  ],
)

/**
 * The control-plane processes serving the node endpoint, each seen every few seconds. While none
 * is, nodes can't report, so their silence isn't held against them.
 */
export const fleetEndpoints = pgTable('fleet_endpoints', {
  processId: text('process_id').primaryKey(),
  startedAt: ts('started_at').notNull(),
  seenAt: ts('seen_at').notNull(),
})

/** What the fleet did and saw, in order, for operators. Kept 90 days. */
export const fleetEvents = pgTable(
  'fleet_events',
  {
    id: bigint('id', { mode: 'number' }).primaryKey().generatedAlwaysAsIdentity(),
    at: ts('at').notNull().defaultNow(),
    kind: text('kind').notNull(),
    nodeId: uuid('node_id'),
    workload: text('workload'),
    epoch: bigint('epoch', { mode: 'number' }),
    data: jsonb('data').$type<Record<string, unknown>>().notNull().default({}),
  },
  (t) => [
    index('fleet_events_at').on(t.at),
    index('fleet_events_node').on(t.nodeId, t.at),
    index('fleet_events_workload').on(t.workload, t.at),
  ],
)
