// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/**
 * The calls the fleet runtime makes to a node's workload API (pryxd's docs/protocol.md): each
 * one's method, path, epoch, headers and timeout, and nothing else. A node's refusal or silence
 * comes back as `NodeRefused` or `NodeUnreachable`, untouched.
 *
 * It decides nothing: what the ledger records around a call is the caller's (`ledger.ts`,
 * `admission.ts`), and how an archive is offered in parts is `transfers.ts`'s.
 */

import nodeSchema from '../../openapi/node-reads.openapi.json'

import { HOUR_MS, MINUTE_MS, SECOND_MS } from './durations.ts'
import { CONFIRM_DELETE_HEADER, type NodeAddress, type NodeClient } from './node-client.ts'
import type { NodeRow } from './registry.ts'
import type { RuntimeSpec } from './runtime-model.ts'
import type {
  DeleteResponse,
  EnsureResponse,
  ExportResponse,
  FenceResponse,
  PowerResponse,
  RestoreResponse,
  SnapshotResponse,
  WorkloadSpec,
  WorkloadView,
} from './wire.ts'

/**
 * Teardown (fence, delete) stops a running copy gracefully first, which takes the spec's grace
 * plus the node's margin: a shorter wait would give up on a delete the node goes on to finish.
 */
export const TEARDOWN_MS = 10 * MINUTE_MS
export const TRANSFER_MS = 3 * HOUR_MS
/** A first PUT may pull the image, which the node gives this long (`pull_timeout_seconds`). */
const FIRST_PUT_MS = 20 * MINUTE_MS
/**
 * A wake's start: a resume the node gives up on after its `resume_timeout_seconds` (10), then the
 * kill and a fresh start on the copy's data.
 */
const WAKE_START_MS = 30 * SECOND_MS
/**
 * A wake's PUT beyond the spec's grace: a changed spec stops the copy gracefully, then makes it
 * again, usually on an image the node holds. The node finishes a PUT its caller stopped waiting
 * for, so the wake's retry finds it done rather than the wake waiting out a first pull.
 */
const WAKE_PUT_MS = 2 * MINUTE_MS
/** A poll's read: one the node doesn't answer this soon is asked again. */
const POLL_MS = 5 * SECOND_MS

/** A node as the runtime dials it. */
type Dialled = Pick<NodeRow, 'id' | 'api_address'> & Partial<Pick<NodeRow, 'features'>>

export const address = (node: Dialled): NodeAddress => ({
  id: node.id,
  apiAddress: node.api_address,
})

function toWorkloadSpec(spec: RuntimeSpec, features: readonly string[]): WorkloadSpec {
  const { cpuReservationMillis, ...resources } = spec.resources
  return {
    image: spec.image,
    ...(features.includes('wasm-packages') && spec.packages ? { packages: [...spec.packages] } : {}),
    ...(spec.agent ? { agent: spec.agent, sparePorts: spec.sparePorts } : {}),
    ...(features.includes('network-policy') && spec.network ? { network: spec.network } : {}),
    ...(features.includes('workload-options') && spec.workdir ? { workdir: spec.workdir } : {}),
    ...(features.includes('workload-options') && spec.user ? { user: spec.user } : {}),
    ...(features.includes('egress-hostnames') && spec.egress ? { egress: spec.egress } : {}),
    ...(spec.entrypoint ? { entrypoint: [...spec.entrypoint] } : {}),
    env: { ...spec.env },
    secrets: { ...spec.secrets },
    resources: {
      ...resources,
      ...(features.includes('cpu-reservation') && cpuReservationMillis != null
        ? { cpuReservationMillis }
        : {}),
    },
    storage: { mountPath: spec.storage.mountPath, sizeGb: spec.storage.sizeGb },
    ports: spec.ports.map((p) => ({
      name: p.name,
      containerPort: p.port,
      protocol: p.protocol,
      audience: [...p.audience],
    })),
    stop: {
      signal: spec.stop.signal,
      timeoutSeconds: spec.stop.timeoutSeconds,
    },
    // Fly's and Docker's runtimes start a failing server again three times; pryxd does it
    // itself, and only while its execution lease lasts: never a fenced copy, nor long without the
    // control plane. After a host reboot it starts again what was running, once the control
    // plane's answer has fenced its copies of sandboxes placed elsewhere meanwhile and granted it a
    // lease.
    restart:
      spec.restart ??
      (nodeSchema.components.schemas.WorkloadSpec.properties.restart.default as WorkloadSpec['restart']),
    labels: { ...spec.labels },
    // A change that names the same, sent to a suspended copy, waits for its next boot (`deferred`).
    ...(spec.resumesOn === undefined ? {} : { resumesOn: spec.resumesOn }),
  }
}

export class WorkloadApi {
  readonly #nodes: Pick<NodeClient, 'call'>

  constructor(
    nodes: Pick<NodeClient, 'call'>,
    readonly beforeStart?: (nodeId: string, workload: string, epoch: number) => Promise<void>,
  ) {
    this.#nodes = nodes
  }

  /** The node's copy of the workload, as it is now; `poll` for a read asked again until a deadline. */
  view(node: Dialled, key: string, poll = false): Promise<WorkloadView> {
    return this.#nodes.call<WorkloadView>(address(node), 'GET', `/v1/workloads/${key}`, {
      ...(poll ? { timeoutMs: POLL_MS } : {}),
    })
  }

  /**
   * The spec to the node. `wake` for a copy that exists, being woken; `ifMatch` replaces only the
   * spec the node's digest names, so an apply can't overwrite a newer one.
   */
  async put(
    node: Dialled,
    key: string,
    spec: RuntimeSpec,
    epoch: number,
    options: { wake?: boolean; ifMatch?: string | null } = {},
  ): Promise<EnsureResponse> {
    return this.#nodes.call<EnsureResponse>(address(node), 'PUT', `/v1/workloads/${key}`, {
      body: toWorkloadSpec(spec, node.features ?? []),
      epoch,
      ...(options.ifMatch ? { headers: { 'if-match': `"${options.ifMatch}"` } } : {}),
      timeoutMs: options.wake ? spec.stop.timeoutSeconds * SECOND_MS + WAKE_PUT_MS : FIRST_PUT_MS,
    })
  }

  /** A power command; `wake` for a start that wakes the copy, resumed or booted. */
  async power(
    node: Dialled,
    key: string,
    epoch: number,
    verb: 'start' | 'stop' | 'kill' | 'suspend',
    wake = false,
  ): Promise<PowerResponse> {
    if (verb === 'start') await this.beforeStart?.(node.id, key, epoch)
    return this.#nodes.call<PowerResponse>(address(node), 'POST', `/v1/workloads/${key}/${verb}`, {
      epoch,
      timeoutMs: verb === 'stop' ? TEARDOWN_MS : wake ? WAKE_START_MS : MINUTE_MS,
    })
  }

  snapshot(
    node: Dialled,
    key: string,
    epoch: number,
    body: { id: string; quiesced: boolean },
  ): Promise<SnapshotResponse> {
    return this.#nodes.call<SnapshotResponse>(address(node), 'POST', `/v1/workloads/${key}/snapshots`, {
      body,
      epoch,
      timeoutMs: TRANSFER_MS,
    })
  }

  /** An upload or export of an archive, to where `body` says. */
  send(node: Dialled, path: string, body: Record<string, unknown>, epoch?: number): Promise<ExportResponse> {
    return this.#nodes.call<ExportResponse>(address(node), 'POST', path, {
      body,
      ...(epoch === undefined ? {} : { epoch }),
      timeoutMs: TRANSFER_MS,
    })
  }

  dropSnapshot(node: Dialled, key: string, localId: string): Promise<unknown> {
    return this.#nodes.call(address(node), 'DELETE', `/v1/workloads/${key}/snapshots/${localId}`, {
      timeoutMs: TEARDOWN_MS,
    })
  }

  restore(
    node: Dialled,
    key: string,
    epoch: number,
    source: { snapshot: string } | { url: string; sha256: string | null },
  ): Promise<RestoreResponse> {
    const body =
      'snapshot' in source
        ? { snapshot: source.snapshot }
        : {
            url: source.url,
            ...(source.sha256 ? { sha256: source.sha256 } : {}),
          }
    return this.#nodes.call<RestoreResponse>(address(node), 'POST', `/v1/workloads/${key}/restore`, {
      body,
      epoch,
      timeoutMs: TRANSFER_MS,
    })
  }

  /** Compute goes; the data stays on the node (`retained`). */
  deleteKeepingData(node: Dialled, key: string, epoch: number): Promise<DeleteResponse> {
    return this.#nodes.call<DeleteResponse>(address(node), 'DELETE', `/v1/workloads/${key}?data=keep`, {
      epoch,
      timeoutMs: TEARDOWN_MS,
    })
  }

  /** Compute and data go, into the node's trash, confirmed by naming the workload again. */
  deleteWithData(node: Dialled, key: string, epoch: number): Promise<DeleteResponse> {
    return this.#nodes.call<DeleteResponse>(address(node), 'DELETE', `/v1/workloads/${key}?data=delete`, {
      epoch,
      headers: { [CONFIRM_DELETE_HEADER]: key },
      timeoutMs: TEARDOWN_MS,
    })
  }

  /** The workload is at `currentEpoch` elsewhere: an older copy here stops and never runs again. */
  fence(node: Dialled, key: string, currentEpoch: number): Promise<FenceResponse> {
    return this.#nodes.call<FenceResponse>(address(node), 'POST', `/v1/workloads/${key}/fence`, {
      body: { currentEpoch },
      timeoutMs: TEARDOWN_MS,
    })
  }
}
