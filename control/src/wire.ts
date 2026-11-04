// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/**
 * pryxd's wire types, as the control plane writes and reads them. They are generated from the
 * node's own Rust types (each cubepals/pryx release carries them as *.openapi.json, which
 * `bun scripts/blocklyd.ts bump` pins here and `bun run openapi:generate` makes generated/ from), so a field the node renames, retypes or
 * makes nullable fails to compile here instead of on the wire. `Reads` is what the node reads: its
 * API's requests and the answers to its own; `Writes` is what it writes.
 *
 * This file names them for the rest of the adapter, which never imports generated/ itself. Where
 * the control plane produces a type the node reads more loosely than it is sent, a narrowing here
 * holds the control plane to what it means to send.
 */

import type { components as NodeReads } from './generated/node-reads.ts'
import type { components as NodeWrites } from './generated/node-writes.ts'

type Reads = NodeReads['schemas']
type Writes = NodeWrites['schemas']

export type WorkloadSpec = Reads['WorkloadSpec']
export type WorkloadState = Writes['WorkloadState']
/**
 * A workload's view, as far as the control plane reads it and its tests' stand-in nodes answer: the
 * node sends more (health, resources, storage, labels, issues, timestamps, each port's protocol),
 * which a field picked here brings in when something needs it.
 */
export type WorkloadView = Pick<
  Writes['WorkloadView'],
  | 'id'
  | 'generation'
  | 'epoch'
  | 'supersededBy'
  | 'specDigest'
  | 'image'
  | 'state'
  | 'exit'
  | 'restartCount'
  | 'lastFailureAt'
  | 'startedAt'
  | 'finishedAt'
  | 'changedAt'
  | 'locate'
> & { ports: Array<Omit<Writes['PortView'], 'protocol'>> }
export type EnsureResponse = Writes['EnsureResponse']
export type PowerResponse = Writes['PowerResponse']
export type FenceResponse = Writes['FenceResponse']
export type DeleteResponse = Writes['DeleteResponse']
export type ExecResponse = Writes['ExecResponse']
export type ExportResponse = Writes['ExportResponse']
export type RestoreResponse = Writes['RestoreResponse']
export type SnapshotResponse = Writes['SnapshotResponse']
export type LogRecord = Writes['LogRecord']

export type NodeCapacity = Writes['NodeCapacity']
export type NodeAddresses = Writes['NodeAddresses']
export type NodeFacts = Writes['NodeFacts']
export type NodeIssue = Writes['Issue']
export type WorkloadReport = Writes['WorkloadReport']
export type EnrollRequest = Writes['EnrollRequest']
export type EnrollResponse = Reads['EnrollResponse']
export type HeartbeatRequest = Writes['HeartbeatRequest']
export type UpgradeOffer = Reads['UpgradeOffer']
export type Fence = Reads['Fence']
/**
 * The node takes `lifecycle` as any string, so a newer control plane can't break its heartbeats;
 * this one sends only these.
 */
export type HeartbeatResponse = Omit<Reads['HeartbeatResponse'], 'lifecycle'> & {
  lifecycle: 'active' | 'draining' | 'lost' | 'retired'
}
export type RenewRequest = Writes['RenewRequest']
export type RenewResponse = Reads['RenewResponse']
