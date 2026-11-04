// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Node and object-store doubles used by fleet runtime regression tests. */

import { type CallOptions, type NodeAddress, NodeRefused } from '../src/node-client.ts'
import type { WorkloadReport, WorkloadState, WorkloadView } from '../src/wire.ts'
import { MemoryStore } from './memory-store.ts'
import { report } from './nodes.ts'

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

/**
 * pryxd as the runtime calls it: the copies each node holds, what reaches the store, and every
 * call. `once` has the next call that matches run something first, which may throw instead.
 */
export class Nodes {
  readonly calls: string[] = []
  readonly copies = new Map<string, Copy>()
  /** What any archive a node packs holds, and the most it sends in one PUT; larger goes in parts. */
  archive = Buffer.from('data!')
  maxPut = Number.POSITIVE_INFINITY
  readonly #store: MemoryStore
  readonly #hooks: Array<{ match: RegExp; run: () => Promise<void> }> = []
  #port = 30_000

  constructor(store: MemoryStore) {
    this.#store = store
  }

  once(match: RegExp, run: () => Promise<void>): void {
    this.#hooks.push({ match, run })
  }

  async call<T>(node: NodeAddress, method: string, path: string, options: CallOptions = {}): Promise<T> {
    const line = `${method} ${path}`
    this.calls.push(`${node.id} ${line}`)
    const at = this.#hooks.findIndex((hook) => hook.match.test(line))
    if (at >= 0) await this.#hooks.splice(at, 1)[0]?.run()
    return this.#answer(node.id, method, path, options) as T
  }

  async probe() {
    return { tcp: 'open' as const, api: 'ok', ms: 1 }
  }

  close(): void {}

  /** What a node's heartbeat says of the copies it holds. */
  reports(node: string): WorkloadReport[] {
    return [...this.copies]
      .filter(([where]) => where.startsWith(`${node}/`))
      .map(([where, copy]) =>
        report(where.slice(node.length + 1), copy.epoch, {
          state: copy.state,
          ports: copy.ports,
        }),
      )
  }

  #answer(node: string, method: string, path: string, options: CallOptions): unknown {
    const [, workload = '', rest = ''] = /^\/v1\/workloads\/([^/?]+)(.*)$/.exec(path) ?? []
    const where = `${node}/${workload}`
    const copy = this.copies.get(where)
    const body = (options.body ?? {}) as {
      url?: string
      id?: string
      parts?: { partSize: number; urls: string[] }
    }
    const power = (state: WorkloadState) => {
      if (copy !== undefined) copy.state = state
      return {
        changed: true,
        forced: false,
        workload: this.#view(workload, copy),
      }
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
    if (method === 'POST' && (rest === '/export' || /^\/snapshots\/[^/]+\/upload$/.test(rest))) {
      const bytes = this.archive
      const sent = {
        sizeBytes: bytes.length,
        sha256: 'a'.repeat(64),
        format: 'tar.gz',
        entries: 1,
        durationMs: 1,
      }
      const refuse = (limitBytes: number) =>
        new NodeRefused(422, 'archive_too_large', 'too large', {
          sizeBytes: bytes.length,
          limitBytes,
        })
      if (bytes.length <= this.maxPut) {
        // The URL is the store's own for one object: what lands there is what the node sent.
        const url = body.url ?? ''
        this.#store.objects.set(decodeURIComponent(url.slice(url.lastIndexOf('/') + 1)), bytes)
        return sent
      }
      // As pryxd does: in parts when they are offered and are enough, else refused unsent.
      const parts = body.parts
      if (parts === undefined) throw refuse(this.maxPut)
      const needed = Math.max(1, Math.ceil(bytes.length / parts.partSize))
      if (needed > parts.urls.length) throw refuse(parts.partSize * parts.urls.length)
      const put = parts.urls.slice(0, needed).map((url, index) => {
        const [, upload = '', number = ''] = /__parts\/([^/]+)\/(\d+)$/.exec(url) ?? []
        const chunk = bytes.subarray(index * parts.partSize, (index + 1) * parts.partSize)
        return {
          number: index + 1,
          etag: this.#store.putPart(upload, Number(number), chunk),
        }
      })
      return { ...sent, parts: put }
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
      this.copies.delete(where)
      return {
        existed: copy !== undefined,
        removedContainer: true,
        data: 'trashed',
        trashPath: null,
      }
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
        containerPort: 8080,
        endpoints: { edge: [], control: [] },
      })),
      locate: { containerName: workload, containerId: null, dataDir: '/data' },
    }
  }
}
