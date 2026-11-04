// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Node and object-store doubles for the fleet flow tests; no tests run on import. */
import { afterAll, beforeAll, describe, expect, test } from 'bun:test'
import { randomUUID } from 'node:crypto'
import type { Db } from '@blockly/db'
import { sql } from 'drizzle-orm'
import {
  type ProgressSink,
  RuntimeFull,
  type RuntimeKey,
  type RuntimeSpec,
  runtimeKey,
  type SnapshotHandle,
} from '../../app/ports/runtime.ts'
import {
  beat,
  capacity,
  type EnrolledNode,
  enrollNode,
  freshDatabase,
  hasDatabase,
  registryOptions,
  report,
  testCa,
} from '../../testing/fleet.ts'
import { MemoryStore } from '../../testing/memory-store.ts'
import type { FleetCa } from './ca.ts'
import { TEARDOWN_MS, TRANSFER_MS } from './fleet-runtime/workload-api.ts'
import { FleetRuntime } from './fleet-runtime.ts'
import { decodeHandle, encodeSnapshot } from './handle.ts'
import { THRESHOLDS } from './health.ts'
import {
  type CallOptions,
  type NodeAddress,
  type NodeClient,
  NodeRefused,
  NodeUnreachable,
} from './node-client.ts'
import { DEFAULTS } from './placement.ts'
import { drain, heartbeat, nodeSummaries } from './registry.ts'
import { rows } from './sql.ts'
import { FleetStore } from './store.ts'
import type { WorkloadReport, WorkloadState, WorkloadView } from './wire.ts'

/** A store whose deletes fail while `failing` is set, as one that is down does. */
export class FlakyStore extends MemoryStore {
  failing = false

  override async delete(key: string): Promise<void> {
    if (this.failing) throw new Error('the store is down')
    await super.delete(key)
  }
}

interface Copy {
  epoch: number
  state: WorkloadState
  ports: Record<string, number>
}

interface Call {
  node: string
  method: string
  path: string
  epoch: number | null | undefined
  timeoutMs: number | undefined
  headers: Record<string, string> | undefined
  body: unknown
}

/** blocklyd as the runtime calls it: one PUT carries any archive here, so nothing goes in parts. */
export class Nodes {
  readonly calls: Call[] = []
  readonly copies = new Map<string, Copy>()
  readonly #store: MemoryStore
  readonly #hooks: Array<{ match: RegExp; run: (answer: () => unknown) => Promise<unknown> }> = []
  #port = 40_000

  constructor(store: MemoryStore) {
    this.#store = store
  }

  /** `run` before the next call `match` finds is answered: what it returns, if anything, is the answer. */
  once(match: RegExp, run: (answer: () => unknown) => Promise<unknown>): void {
    this.#hooks.push({ match, run })
  }

  async call<T>(node: NodeAddress, method: string, path: string, options: CallOptions = {}): Promise<T> {
    this.calls.push({
      node: node.id,
      method,
      path,
      epoch: options.epoch,
      timeoutMs: options.timeoutMs,
      headers: options.headers,
      body: options.body,
    })
    const line = `${method} ${path}`
    const at = this.#hooks.findIndex((hook) => hook.match.test(line))
    const answer = () => this.#answer(node.id, method, path, options)
    return ((at >= 0 ? await this.#hooks.splice(at, 1)[0]?.run(answer) : undefined) ?? answer()) as T
  }

  async probe() {
    return { tcp: 'open' as const, api: 'ok', ms: 1 }
  }

  close(): void {}

  reports(node: string): WorkloadReport[] {
    return [...this.copies]
      .filter(([where]) => where.startsWith(`${node}/`))
      .map(([where, copy]) =>
        report(where.slice(node.length + 1), copy.epoch, { state: copy.state, ports: copy.ports }),
      )
  }

  #answer(node: string, method: string, path: string, options: CallOptions): unknown {
    const [, workload = '', rest = ''] = /^\/v1\/workloads\/([^/?]+)(.*)$/.exec(path) ?? []
    const where = `${node}/${workload}`
    const copy = this.copies.get(where)
    const body = (options.body ?? {}) as { url?: string; id?: string }
    const power = (state: WorkloadState) => {
      if (copy !== undefined) copy.state = state
      return { changed: true, forced: false, workload: this.#view(workload, copy) }
    }
    if (method === 'GET' && rest === '') return this.#view(workload, copy)
    if (method === 'PUT') {
      const made = {
        epoch: options.epoch ?? 0,
        state: copy?.state ?? 'created',
        ports: copy?.ports ?? { game: this.#port++ },
      }
      this.copies.set(where, made)
      return {
        outcome: copy ? 'replaced' : 'created',
        restarted: false,
        workload: this.#view(workload, made),
      }
    }
    if (method === 'POST' && rest === '/start') return power('running')
    if (method === 'POST' && (rest === '/stop' || rest === '/kill')) return power('stopped')
    if (method === 'POST' && rest === '/fence') return { ...power('fenced'), stopped: false }
    if (method === 'POST' && rest === '/snapshots')
      return {
        created: true,
        snapshot: {
          id: `local-${body.id}`,
          workload,
          epoch: options.epoch ?? null,
          createdAt: new Date().toISOString(),
          sizeBytes: 5,
          files: 1,
          method: 'copy',
          quiesced: true,
          specDigest: 'd',
          durationMs: 1,
        },
      }
    if (method === 'POST' && (rest === '/export' || /^\/snapshots\/[^/]+\/upload$/.test(rest))) {
      const url = body.url ?? ''
      this.#store.objects.set(decodeURIComponent(url.slice(url.lastIndexOf('/') + 1)), Buffer.from('world'))
      return { sizeBytes: 5, sha256: 'a'.repeat(64), format: 'tar.gz', entries: 1, durationMs: 1 }
    }
    if (method === 'POST' && rest === '/restore')
      return {
        sizeBytes: 5,
        sha256: null,
        entries: 1,
        skipped: 0,
        unpackedBytes: 5,
        previousData: null,
        durationMs: 1,
      }
    if (method === 'DELETE' && rest.startsWith('/snapshots/')) return { existed: true }
    if (method === 'DELETE') {
      if (rest === '?data=delete') this.copies.delete(where)
      return { existed: copy !== undefined, removedContainer: true, data: 'trashed', trashPath: null }
    }
    throw new Error(`this node doesn't answer ${method} ${path}`)
  }

  #view(workload: string, copy: Copy | undefined): WorkloadView {
    if (copy === undefined) throw new NodeRefused(404, 'not_found', 'no such workload', null)
    return {
      id: workload,
      generation: 1,
      epoch: copy.epoch,
      supersededBy: null,
      specDigest: 'd',
      image: 'alpine:3.22',
      state: copy.state,
      exit: null,
      restartCount: 0,
      lastFailureAt: null,
      startedAt: null,
      finishedAt: null,
      changedAt: new Date().toISOString(),
      ports: Object.entries(copy.ports).map(([name, hostPort]) => ({
        name,
        hostPort,
        containerPort: 25565,
        endpoints: { edge: [], control: [] },
      })),
      locate: { containerName: workload, containerId: null, dataDir: '/data' },
    }
  }
}

