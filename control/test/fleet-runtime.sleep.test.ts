// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/**
 * FleetRuntime's sleep and wake against a pryxd in memory that suspends as 0.3.2 does: a sleep
 * suspends or stops, a wake resumes or boots, a spec is sent only when it changed, a change that
 * names the suspended copy's `resumesOn` waits for its next boot, and an apply names the digest it
 * replaces. fleet-runtime.e2e.test.ts runs a sleep and a wake against the real one.
 */

import { afterAll, beforeAll, describe, expect, test } from 'bun:test'
import { createHash, randomUUID } from 'node:crypto'
import { sql } from 'drizzle-orm'
import { StatusCodes } from 'http-status-codes'
import { FleetStore } from '../src/archive-transport.ts'
import type { FleetCa } from '../src/ca.ts'
import type { Db } from '../src/database.ts'
import { SECOND_MS } from '../src/durations.ts'
import { decodeHandle } from '../src/handle.ts'
import { THRESHOLDS } from '../src/health.ts'
import {
  type CallOptions,
  type NodeAddress,
  type NodeClient,
  NodeRefused,
  NodeUnreachable,
} from '../src/node-client.ts'
import { DEFAULTS } from '../src/placement.ts'
import { heartbeat, nodeSummaries } from '../src/registry.ts'
import { FleetRuntime } from '../src/runtime-engine.ts'
import { type ProgressSink, type RuntimeKey, type RuntimeSpec, runtimeKey } from '../src/runtime-model.ts'
import { rows } from '../src/sql.ts'
import type { WorkloadReport, WorkloadState, WorkloadView } from '../src/wire.ts'
import { MemoryStore } from './memory-store.ts'
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
} from './nodes.ts'

interface Copy {
  epoch: number
  state: WorkloadState
  digest: string
  resumesOn: string | null
  pending: { digest: string; resumesOn: string | null } | null
}

interface Call {
  node: string
  line: string
  timeoutMs: number | undefined
  headers: Record<string, string> | undefined
}

/** The node's digest: over the spec without secret values, as pryxd's leaves them out. */
const nodeDigest = (body: { secrets?: object; resumesOn?: string }) => {
  const { secrets: _secrets, resumesOn: _resumesOn, ...rest } = body
  return `sha256:${createHash('sha256').update(JSON.stringify(rest)).digest('hex')}`
}

class Nodes {
  readonly calls: Call[] = []
  readonly copies = new Map<string, Copy>()
  /** Calls that never answer until `signal` ends, and those the node is unreachable for. */
  readonly hang: string[] = []
  readonly unreachable: string[] = []

  async call<T>(node: NodeAddress, method: string, path: string, options: CallOptions = {}): Promise<T> {
    const line = `${method} ${path.replace(/^\/v1\/workloads\/[^/?]+/, '')}`
    this.calls.push({
      node: node.id,
      line,
      timeoutMs: options.timeoutMs,
      headers: options.headers,
    })
    if (this.unreachable[0] === line) {
      this.unreachable.shift()
      throw new NodeUnreachable('refused', 'connection refused')
    }
    if (this.hang[0] === line) {
      this.hang.shift()
      return new Promise<T>(() => {})
    }
    const workload = /^\/v1\/workloads\/([^/?]+)/.exec(path)?.[1] ?? ''
    return this.#answer(`${node.id}/${workload}`, workload, line, options) as T
  }

  reports(node: string): WorkloadReport[] {
    return [...this.copies]
      .filter(([where]) => where.startsWith(`${node}/`))
      .map(([where, copy]) =>
        report(where.slice(node.length + 1), copy.epoch, {
          state: copy.state,
          specDigest: copy.digest,
        }),
      )
  }

  #answer(where: string, workload: string, line: string, options: CallOptions): unknown {
    const copy = this.copies.get(where)
    if (line === 'GET ') return this.#view(workload, copy)
    if (line === 'PUT ') return this.#put(where, workload, copy, options)
    if (copy === undefined)
      throw new NodeRefused(StatusCodes.NOT_FOUND, 'not_found', 'no such workload', null)
    const power = (state: WorkloadState, more: { resumed?: boolean } = {}) => {
      const changed = copy.state !== state
      copy.state = state
      return {
        changed,
        forced: false,
        ...more,
        workload: this.#view(workload, copy),
      }
    }
    if (line === 'POST /suspend') {
      if (copy.state !== 'running' && copy.state !== 'suspended')
        throw new NodeRefused(StatusCodes.CONFLICT, 'not_running', 'not running', null)
      return power('suspended')
    }
    if (line === 'POST /start') return this.#start(copy, power)
    if (line === 'POST /stop' || line === 'POST /kill') return power('stopped')
    throw new Error(`this node doesn't answer ${line}`)
  }

  /** A start resumes a suspended copy; any other boots it, putting a deferred spec in place first. */
  #start(copy: Copy, power: (state: WorkloadState, more?: { resumed?: boolean }) => unknown) {
    if (copy.state === 'suspended') return power('running', { resumed: true })
    if (copy.pending !== null) {
      copy.digest = copy.pending.digest
      copy.resumesOn = copy.pending.resumesOn
      copy.pending = null
    }
    return power('running')
  }

  #put(where: string, workload: string, copy: Copy | undefined, options: CallOptions) {
    const body = options.body as { secrets?: object; resumesOn?: string }
    const digest = nodeDigest(body)
    const resumesOn = body.resumesOn ?? null
    const ifMatch = options.headers?.['if-match']
    if (ifMatch !== undefined && ifMatch !== `"${copy?.digest}"`)
      throw new NodeRefused(StatusCodes.PRECONDITION_FAILED, 'precondition_failed', 'the spec changed', {
        currentSpecDigest: copy?.digest,
      })
    if (copy === undefined) {
      const made: Copy = {
        epoch: options.epoch ?? 0,
        state: 'created',
        digest,
        resumesOn,
        pending: null,
      }
      this.copies.set(where, made)
      return {
        outcome: 'created',
        restarted: false,
        workload: this.#view(workload, made),
      }
    }
    const answer = (outcome: string) => ({
      outcome,
      restarted: false,
      workload: this.#view(workload, copy),
    })
    if (digest === copy.digest) {
      copy.pending = null
      return answer('unchanged')
    }
    if (copy.state === 'suspended' && resumesOn !== null && resumesOn === copy.resumesOn) {
      copy.pending = { digest, resumesOn }
      return answer('deferred')
    }
    Object.assign(copy, { digest, resumesOn, pending: null })
    if (copy.state === 'suspended') copy.state = 'stopped'
    return answer('replaced')
  }

  #view(workload: string, copy: Copy | undefined): WorkloadView {
    if (copy === undefined)
      throw new NodeRefused(StatusCodes.NOT_FOUND, 'not_found', 'no such workload', null)
    return {
      id: workload,
      generation: 1,
      epoch: copy.epoch,
      supersededBy: null,
      specDigest: copy.digest,
      image: 'alpine:3.22',
      state: copy.state,
      exit: null,
      restartCount: 0,
      lastFailureAt: null,
      startedAt: null,
      finishedAt: null,
      changedAt: new Date().toISOString(),
      ports: [
        {
          name: 'game',
          hostPort: 40_000,
          containerPort: 8080,
          endpoints: { edge: [], control: [] },
        },
      ],
      locate: { containerName: workload, containerId: null, dataDir: '/data' },
    }
  }
}

const SPEC: RuntimeSpec = {
  image: 'alpine:3.22',
  env: { MOTD: 'Hello' },
  secrets: { RCON_PASSWORD: 'one' },
  resources: { memoryMb: 1024 },
  storage: { mountPath: '/data', sizeGb: 1 },
  ports: [
    {
      name: 'game',
      port: 8080,
      protocol: 'tcp',
      audience: ['edge', 'control'],
    },
  ],
  stop: { signal: 'SIGTERM', timeoutSeconds: 5 },
  labels: {},
  resumesOn: 'as-before',
}

const FEATURES = [
  'placement-epochs',
  'data-transfer',
  'local-snapshots',
  'export-exclude',
  'execution-lease',
  'multipart-upload',
  'suspend',
]

let db: Db
let drop: () => Promise<void>
let ca: FleetCa
let store: MemoryStore
let nodes: Nodes
let suspending: FleetRuntime
let stopping: FleetRuntime
let seq = 0

const pulse = (n: EnrolledNode, features = FEATURES) =>
  heartbeat(
    db,
    registryOptions(),
    n.id,
    n.sha,
    beat(n, 'session', ++seq, nodes.reports(n.id), {
      features,
      capacity: capacity({ allocatableMemoryMb: 4096 }),
    }),
  )

/** A server running on a node of its own region, which lists `features`. */
async function running(name: string, features = FEATURES) {
  const node = await enrollNode(db, ca, name, { region: name })
  await pulse(node, features)
  await suspending.refreshRegistry()
  const key: RuntimeKey = runtimeKey(randomUUID())
  const handle = await suspending.ensureProvisioned(key, { regionKey: name }, SPEC, quiet)
  await pulse(node, features)
  return { node, key, handle, since: nodes.calls.length }
}

const callsSince = (from: number) => nodes.calls.slice(from).map((c) => `${c.line} t${c.timeoutMs ?? '-'}`)

function recorder(): { sink: ProgressSink; seen: string[] } {
  const seen: string[] = []
  return {
    seen,
    sink: {
      step: async (step) => {
        seen.push(step)
      },
      handle: async (handle) => {
        seen.push(`handle e${decodeHandle(handle).epoch}`)
      },
    },
  }
}
const quiet: ProgressSink = { step: async () => {}, handle: async () => {} }

const digests = async (key: string) =>
  (
    await rows<{ spec_digest: string | null; sent_digest: string | null }>(
      db,
      sql`SELECT spec_digest, sent_digest FROM fleet_placements WHERE workload = ${key}`,
    )
  )[0]
const claimed = async (node: string) =>
  (await nodeSummaries(db, THRESHOLDS)).find((n) => n.id === node)?.running.memoryMb

beforeAll(async () => {
  if (!hasDatabase) return
  ;({ db, drop } = await freshDatabase())
  ca = await testCa()
  store = new MemoryStore()
  await store.start()
  nodes = new Nodes()
  const options = {
    db,
    nodes: nodes as unknown as NodeClient,
    store: new FleetStore(store, 'test'),
    deployment: 'test',
    regionMap: {},
    placement: DEFAULTS,
    cpuMillisPerGb: 250,
    thresholds: THRESHOLDS,
    log: () => {},
  }
  suspending = new FleetRuntime({ ...options, suspend: true })
  stopping = new FleetRuntime(options)
})
afterAll(async () => {
  store?.close()
  await drop?.()
})

describe.skipIf(!hasDatabase)('FleetRuntime asleep, with pryxd in memory', () => {
  test('with the switch off, a sleep stops and gives its room back', async () => {
    const { node, handle, since } = await running('off')
    await stopping.refreshRegistry()
    expect(await stopping.sleep(handle)).toBe('stopped')
    expect(callsSince(since)).toEqual(['POST /stop t600000'])
    await pulse(node)
    expect(await claimed(node.id)).toBe(0)
  })

  test('on a node without the feature, a sleep stops', async () => {
    const { handle, since } = await running('no-feature', FEATURES.slice(0, -1))
    expect(await suspending.sleep(handle)).toBe('stopped')
    expect(callsSince(since)).toEqual(['POST /stop t600000'])
  })

  test('a copy not running is stopped, as one the node won’t suspend', async () => {
    const { handle, since } = await running('not-running')
    await suspending.stop(handle)
    expect(await suspending.sleep(handle)).toBe('stopped')
    expect(callsSince(since)).toEqual(['POST /stop t600000', 'POST /suspend t60000', 'POST /stop t600000'])
  })

  test('a sleep suspends, keeps its room, and the wake resumes it with one call', async () => {
    const { node, key, handle, since } = await running('resume')
    expect(await suspending.sleep(handle)).toBe('suspended')
    expect(await suspending.sleep(handle)).toBe('suspended')
    await pulse(node)
    expect(await claimed(node.id)).toBe(1024)
    expect((await suspending.observe(handle)).state).toBe('stopped')
    const { sink, seen } = recorder()
    const woken = await suspending.ensureProvisioned(key, { regionKey: 'resume' }, SPEC, sink)
    await suspending.waitRunning(woken, AbortSignal.timeout(SECOND_MS))
    expect(woken).toBe(handle)
    expect(callsSince(since)).toEqual(['POST /suspend t60000', 'POST /suspend t60000', 'POST /start t30000'])
    expect(seen).toEqual(['allocating', 'handle e1', 'resuming'])
  })
})

describe.skipIf(!hasDatabase)('FleetRuntime woken, with pryxd in memory', () => {
  test('a change only a start reads waits, asleep, for the next boot', async () => {
    const { node, key, handle, since } = await running('deferred')
    const ran = (await digests(key))?.spec_digest
    await suspending.sleep(handle)
    await pulse(node)
    const changed = { ...SPEC, env: { MOTD: 'Back soon' } }
    const { sink, seen } = recorder()
    await suspending.ensureProvisioned(key, { regionKey: 'deferred' }, changed, sink)
    expect(seen).toEqual(['allocating', 'compute', 'handle e1', 'resuming'])
    expect(callsSince(since).slice(1)).toEqual(['PUT  t125000', 'POST /start t30000'])
    // It runs what it slept on, so the node's digest is unknown until the change boots.
    expect(nodes.copies.get(`${node.id}/${key}`)?.digest).toBe(ran ?? '')
    expect((await digests(key))?.spec_digest).toBeNull()

    await suspending.stop(handle)
    await pulse(node)
    const after = nodes.calls.length
    const boot = recorder()
    await suspending.ensureProvisioned(key, { regionKey: 'deferred' }, changed, boot.sink)
    expect(callsSince(after)).toEqual(['POST /start t30000'])
    expect(boot.seen).toEqual(['allocating', 'handle e1', 'booting'])
    const booted = nodes.copies.get(`${node.id}/${key}`)?.digest
    expect(booted).not.toBe(ran)
    expect((await digests(key))?.spec_digest).toBe(booted ?? '')
  })

  test('a rotated secret is sent, since the node’s digest leaves secrets out', async () => {
    const { key, handle, since } = await running('secret')
    await suspending.stop(handle)
    await suspending.ensureProvisioned(
      key,
      { regionKey: 'secret' },
      { ...SPEC, secrets: { RCON_PASSWORD: 'two' } },
      quiet,
    )
    expect(callsSince(since)).toEqual(['POST /stop t600000', 'PUT  t125000', 'POST /start t30000'])
  })

  test('a start of a copy the node lost sends the spec again, then starts', async () => {
    const { node, key, handle, since } = await running('lost')
    await suspending.stop(handle)
    nodes.copies.delete(`${node.id}/${key}`)
    await suspending.ensureProvisioned(key, { regionKey: 'lost' }, SPEC, quiet)
    expect(callsSince(since)).toEqual([
      'POST /stop t600000',
      'POST /start t30000',
      'PUT  t125000',
      'POST /start t30000',
    ])
  })
})

describe.skipIf(!hasDatabase)('FleetRuntime applying and waiting, with pryxd in memory', () => {
  test('an apply replaces only the spec the ledger last saw', async () => {
    const { node, key, handle, since } = await running('apply')
    const ran = (await digests(key))?.spec_digest
    await suspending.apply(handle, { ...SPEC, env: { MOTD: 'New' } })
    expect(nodes.calls[since]?.headers).toEqual({ 'if-match': `"${ran}"` })
    const copy = nodes.copies.get(`${node.id}/${key}`)
    if (copy !== undefined) copy.digest = 'sha256:elsewhere'
    const refused = suspending.apply(handle, SPEC)
    await expect(refused).rejects.toThrow('the spec changed')
  })

  test('the wait for running asks again past an unreachable node, and ends with its signal', async () => {
    const { handle } = await running('poll')
    await suspending.stop(handle)
    await suspending.start(handle)
    nodes.unreachable.push('GET ')
    const since = nodes.calls.length
    await suspending.waitRunning(handle, AbortSignal.timeout(5 * SECOND_MS))
    expect(callsSince(since)).toEqual(['GET  t5000', 'GET  t5000'])

    nodes.hang.push('GET ')
    const started = Date.now()
    const wait = suspending.waitRunning(handle, AbortSignal.timeout(0.2 * SECOND_MS))
    await expect(wait).rejects.toThrow('The workload did not start in time')
    expect(Date.now() - started).toBeLessThan(2_000)
  })
})
