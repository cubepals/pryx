// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/**
 * Two nodes, each a pryxd with a Docker daemon of its own (Docker in Docker, privileged), and
 * the production adapter placing, moving and rebuilding sandboxes between them. Both nodes share
 * this machine's kernel and disk, so this is two container runtimes and two network namespaces,
 * not two independent hosts: it tests the fleet's logic, not hardware failure.
 *
 * Each node is set up as an operator's pasted line sets one up: `pryxd join <bk1. token>`,
 * which fetches the CA from the node endpoint and works out the node's address. Only this test's
 * policy (alpine, a small host) is added to what it writes. fleet-runtime.e2e.test.ts keeps a
 * hand-written configuration, so both ways stay covered.
 *
 *   gh release download v$(bun scripts/blocklyd.ts version) -R cubepals/pryx -p pryxd   # static
 *   PRYXD_STATIC_BIN=$PWD/pryxd DATABASE_URL=… \
 *     S3_TEST_ENDPOINT=… bun test fleet-two-node.e2e
 *
 * Runs as root, with docker:29-dind and alpine:3.22 present on the host's Docker.
 */

import { afterAll, beforeAll, describe, expect, test } from 'bun:test'
import { spawnSync } from 'node:child_process'
import { randomBytes } from 'node:crypto'
import { existsSync, mkdirSync, writeFileSync } from 'node:fs'
import { createServer } from 'node:net'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { sql } from 'drizzle-orm'
import { FleetStore } from '../src/archive-transport.ts'
import { controlPlaneName, FleetCa } from '../src/ca.ts'
import type { Db } from '../src/database.ts'
import { MINUTE_MS, SECOND_MS } from '../src/durations.ts'
import { decodeHandle } from '../src/handle.ts'
import { type JoinPoint, joinCommand, joinToken } from '../src/join-token.ts'
import { NodeClient, NodeUnreachable, splitAddress } from '../src/node-client.ts'
import { DEFAULTS } from '../src/placement.ts'
import { confirmLost, createToken, drain, holdsMemory, nodeSummaries, undrain } from '../src/registry.ts'
import { FleetRuntime } from '../src/runtime-engine.ts'
import type { ProgressSink, RuntimeHandle, RuntimeSpec, SnapshotHandle } from '../src/runtime-model.ts'
import { RuntimeFull, runtimeKey } from '../src/runtime-model.ts'
import { rows } from '../src/sql.ts'
import { S3ArchiveStore } from '../src/store.ts'
import { freshDatabase, hasDatabase } from '../test/nodes.ts'
import { createTestBucket } from './test-bucket.ts'

const env = process.env
const BIN = env.PRYXD_STATIC_BIN
const S3 = env.S3_TEST_ENDPOINT
const ready = Boolean(
  BIN &&
    existsSync(BIN) &&
    S3 &&
    hasDatabase &&
    existsSync('/var/run/docker.sock') &&
    process.getuid?.() === 0,
)

const DEPLOYMENT = `two${randomBytes(3).toString('hex')}`
const THRESHOLDS = { suspectSeconds: 3, unavailableSeconds: 6 }
const credentials = {
  accessKeyId: env.S3_TEST_ACCESS_KEY_ID ?? 'blockly-test',
  secretAccessKey: env.S3_TEST_SECRET_ACCESS_KEY ?? 'blockly-test-secret',
}
const DIND = 'docker:29-dind'
const NETWORK = `blockly-${DEPLOYMENT}`
/** This test's policy, over what `pryxd join` writes: what may run, and a small host. */
const POLICY = `
[network]
port_quarantine_seconds = 0
[capacity]
reserved_memory_mb = 0
allocatable_memory_mb = 2048
min_free_disk_mb = 256
`

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms))
const progress: ProgressSink = { step: async () => {}, handle: async () => {} }

async function freePort(): Promise<number> {
  return new Promise((resolve, reject) => {
    const server = createServer()
    server.once('error', reject)
    server.listen(0, '127.0.0.1', () => {
      const address = server.address()
      server.close(() => resolve(typeof address === 'object' && address ? address.port : 0))
    })
  })
}

async function until<T>(
  what: string,
  check: () => Promise<T | null | undefined | false>,
  ms = MINUTE_MS,
): Promise<T> {
  const deadline = Date.now() + ms
  for (;;) {
    const value = await check().catch(() => null)
    if (value) return value
    if (Date.now() > deadline) throw new Error(`Timed out waiting for ${what}`)
    await sleep(0.25 * SECOND_MS)
  }
}

function docker(...args: string[]): string {
  const run = spawnSync('docker', args)
  if (run.status !== 0)
    throw new Error(
      `docker ${args.map((value, i) => (args[i - 1] === 'join' ? '[redacted]' : value)).join(' ')}: ${run.stderr.toString().trim()}`,
    )
  return run.stdout.toString().trim()
}

const SCRIPT = [
  'mkdir -p /data/data /data/libraries',
  '[ -f /data/data/level.dat ] || echo fresh > /data/data/level.dat',
  'echo jar > /data/libraries/server.jar',
  "trap 'exit 0' TERM",
  'while true; do sleep 1; done',
].join('; ')

const SPEC: RuntimeSpec = {
  image: 'alpine:3.22',
  entrypoint: ['sh', '-c', SCRIPT],
  env: {},
  secrets: { RCON_PASSWORD: 'two-node' },
  resources: { memoryMb: 128 },
  storage: { mountPath: '/data', sizeGb: 1, reconstructible: ['libraries'] },
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
}

/**
 * The line an operator pastes, but with join.sh read rather than run (on this machine it would
 * install Docker), then pryxd fetched and checked as join.sh does.
 */
function fetchedAsPasted(point: JoinPoint): string {
  return [
    joinCommand(point, 'bk1.e30').replace(/ \| sh -s -- .*$/, ' > "$d/join.sh" && sh -n "$d/join.sh"'),
    `curl -fsS --cacert "$d/ca.pem" '${point.url}/fleet/v1/blocklyd' -o "$d/pryxd"`,
    `curl -fsS --cacert "$d/ca.pem" '${point.url}/fleet/v1/blocklyd.sha256' -o "$d/blocklyd.sha256"`,
    'cd "$d" && sha256sum -c --quiet blocklyd.sha256',
  ].join(' && ')
}

interface Node {
  name: string
  container: string
  ip: string
  id: string
  /** Starts pryxd in the node's container, as its init system would at boot. */
  boot: () => void
}

/** What a scenario below reads of the two nodes the suite set up, and the server it keeps. */
interface TwoNodes {
  db: Db
  runtime: FleetRuntime
  node: Record<'a' | 'b', Node>
  h1: RuntimeHandle
}

/** The sandboxes the suite's s1, s2 and s3 name. */
const KEY = {
  s1: runtimeKey('1b6f1f2e-6a47-4c9a-9a39-2f4ac7e51f10'),
  s2: runtimeKey('2b6f1f2e-6a47-4c9a-9a39-2f4ac7e51f10'),
  s3: runtimeKey('3b6f1f2e-6a47-4c9a-9a39-2f4ac7e51f10'),
}
const nodeOf = (handle: RuntimeHandle) => decodeHandle(handle).node
const ledger = async (db: Db) => new Map((await nodeSummaries(db, THRESHOLDS)).map((n) => [n.id, n]))
const movedForRoom = (db: Db) =>
  rows(db, sql`SELECT 1 FROM fleet_events WHERE kind = 'placement.moved_for_room'`)

/** The fleet acting on the nodes' own answers, run in order after the moves above. */
const ANSWERS: Array<{
  name: string
  ms: number
  run: (t: TwoNodes) => Promise<void>
}> = [
  {
    name: "a wake right after another server's stop on a full node starts in place",
    ms: 180_000,
    run: async ({ db, runtime, node, h1 }) => {
      const home = nodeOf(h1)
      const away = home === node.a.id ? node.b : node.a
      // The stop's answer frees its memory, before the node beats again.
      await runtime.stop(h1)
      expect((await ledger(db)).get(home ?? '')?.running.memoryMb).toBe(0)
      // Another server takes all but 64 MB of the node's 2048 to run.
      await drain(db, away.id, 'e2e')
      const h3 = await runtime.ensureProvisioned(
        KEY.s3,
        { regionKey: 'eu' },
        { ...SPEC, resources: { memoryMb: 1984 } },
        progress,
      )
      await undrain(db, away.id, 'e2e')
      expect(nodeOf(h3)).toBe(home)
      await runtime.waitRunning(h3, AbortSignal.timeout(MINUTE_MS))
      const moves = (await movedForRoom(db)).length
      // It stops, and the sleeping server wakes at once, where it sleeps.
      await runtime.stop(h3)
      const woken = await runtime.ensureProvisioned(KEY.s1, { regionKey: 'eu' }, SPEC, progress)
      expect(woken).toBe(h1)
      expect((await movedForRoom(db)).length).toBe(moves)
      await runtime.waitRunning(h1, AbortSignal.timeout(MINUTE_MS))
      expect((await runtime.exec(h1, ['cat', '/data/data/level.dat'], 10)).stdout.trim()).toBe('slept')
      await runtime.destroy(h3)
    },
  },
  {
    name: 'an apply against a node that went away ends stopped on what it ran, not failed',
    ms: 180_000,
    run: async ({ db, runtime, node, h1 }) => {
      const busy = nodeOf(h1) ?? ''
      const gone = busy === node.a.id ? node.b : node.a
      await drain(db, busy, 'e2e')
      const h2 = await runtime.ensureProvisioned(KEY.s2, { regionKey: 'eu' }, SPEC, progress)
      await undrain(db, busy, 'e2e')
      expect(nodeOf(h2)).toBe(gone.id)
      await runtime.waitRunning(h2, AbortSignal.timeout(MINUTE_MS))
      // The node turns away every call to its API; it still beats, and its copy still runs.
      const api = (await ledger(db)).get(gone.id)?.api_address ?? ''
      const rule = ['INPUT', '-p', 'tcp', '--dport', String(splitAddress(api).port)]
      docker('exec', gone.container, 'iptables', '-I', ...rule, '-j', 'REJECT', '--reject-with', 'tcp-reset')
      try {
        await expect(runtime.apply(h2, { ...SPEC, env: { CHANGED: '1' } })).rejects.toBeInstanceOf(
          NodeUnreachable,
        )
        // What the application does when it can't go back: a stop, killed where it won't.
        await runtime.stop(h2).catch(() => runtime.forceStop(h2))
        // Its claim is given back; the copy holds memory only while its node says it still runs.
        const [placed] = await rows<{ desired_power: string }>(
          db,
          sql`SELECT desired_power FROM fleet_placements WHERE workload = ${KEY.s2}`,
        )
        expect(placed?.desired_power).toBe('stopped')
        const unanswered = await rows<{ data: { verb: string } }>(
          db,
          sql`SELECT data FROM fleet_events WHERE kind = 'power.unanswered' AND workload = ${KEY.s2}`,
        )
        expect(unanswered.map((e) => e.data.verb)).toEqual(['kill'])
      } finally {
        docker(
          'exec',
          gone.container,
          'iptables',
          '-D',
          ...rule,
          '-j',
          'REJECT',
          '--reject-with',
          'tcp-reset',
        )
      }
      // Back in reach, the copy the kill never reached is stopped, as stray compute is.
      await runtime.stop(h2)
      expect(await holdsMemory(db, KEY.s2)).toBe(false)
      await runtime.destroy(h2)
    },
  },
]

describe.skipIf(!ready)('FleetRuntime across two nodes', () => {
  const dir = join(tmpdir(), `blockly-${DEPLOYMENT}`)
  let db: Db
  let drop: () => Promise<void>
  let endpoint: ReturnType<typeof Bun.spawn> | undefined
  let runtime: FleetRuntime
  let nodes: NodeClient
  const node: Record<'a' | 'b', Node> = {} as Record<'a' | 'b', Node>
  const { s1, s2, s3 } = KEY
  let h1: RuntimeHandle
  let lastSnapshot: SnapshotHandle
  let joinPoint: JoinPoint

  const exec = async (handle: RuntimeHandle, command: string) =>
    (await runtime.exec(handle, ['sh', '-c', command], 10)).stdout.trim()
  const other = (id: string | null) => (id === node.a.id ? node.b : node.a)
  const inner = (n: Node, ...args: string[]) => docker('exec', n.container, 'docker', ...args)
  const uploaded = () =>
    until('the snapshot upload', async () => {
      await runtime.uploadPending()
      const states = await rows<{ status: string; error: string | null }>(
        db,
        sql`SELECT status, error FROM fleet_archives WHERE purpose = 'snapshot'`,
      )
      const failed = states.find((s) => s.status === 'failed')
      if (failed) throw new Error(`the upload failed: ${failed.error}`)
      return states.every((s) => s.status === 'ready')
    })

  beforeAll(async () => {
    mkdirSync(dir, { recursive: true })
    const fresh = await freshDatabase()
    ;({ db, drop } = fresh)
    const bucket = `blockly-${DEPLOYMENT}`
    await createTestBucket(S3 ?? '', credentials, bucket)
    // The nodes' network, and where they reach the control plane on it.
    docker('network', 'create', '--subnet', '172.31.250.0/24', NETWORK)
    const gateway = '172.31.250.1'
    // Where the nodes reach the store. S3_TEST_CONTAINER: a store published on loopback only
    // joins the nodes' network.
    let storeForNodes = (S3 ?? '').replace('127.0.0.1', gateway)
    if (env.S3_TEST_CONTAINER) {
      docker('network', 'connect', '--ip', '172.31.250.5', NETWORK, env.S3_TEST_CONTAINER)
      storeForNodes = 'http://172.31.250.5:9000'
    }

    const pems = await FleetCa.generate(DEPLOYMENT)
    const ca = await FleetCa.fromPem(pems.certPem, pems.keyPem)
    writeFileSync(join(dir, 'ca.pem'), ca.pem)
    writeFileSync(join(dir, 'ca.key'), pems.keyPem, { mode: 0o600 })
    const endpointPort = await freePort()
    endpoint = Bun.spawn(
      [
        'node',
        join(import.meta.dir, 'endpoint.e2e.ts'),
        join(dir, 'ca.pem'),
        join(dir, 'ca.key'),
        String(endpointPort),
        DEPLOYMENT,
        gateway,
        gateway,
      ],
      {
        env: { ...env, DATABASE_URL: fresh.url },
        stdout: 'pipe',
        stderr: 'inherit',
      },
    )
    const first = await (endpoint.stdout as ReadableStream<Uint8Array>).getReader().read()
    if (!new TextDecoder().decode(first.value).includes('listening'))
      throw new Error('the node endpoint failed')
    const client = await ca.identity(controlPlaneName(DEPLOYMENT), 'client', 1)
    nodes = new NodeClient({ caPem: ca.pem, certPem: client.certPem, keyPem: client.keyPem }, DEPLOYMENT)
    runtime = new FleetRuntime({
      db,
      nodes,
      store: new FleetStore(
        new S3ArchiveStore({
          endpoint: S3 ?? '',
          runtimeEndpoint: storeForNodes,
          bucket,
          region: 'auto',
          ...credentials,
        }),
        DEPLOYMENT,
      ),
      deployment: DEPLOYMENT,
      regionMap: {},
      placement: DEFAULTS,
      cpuMillisPerGb: 250,
      thresholds: THRESHOLDS,
      log: () => {},
    })

    const image = spawnSync('sh', ['-c', 'docker save alpine:3.22 > image.tar'], { cwd: dir })
    if (image.status !== 0) throw new Error('alpine:3.22 must be on the host')
    joinPoint = {
      url: `https://${gateway}:${endpointPort}`,
      caPem: ca.pem,
      deployment: DEPLOYMENT,
    }
    for (const [name, ip] of [
      ['a', '172.31.250.11'],
      ['b', '172.31.250.12'],
    ] as const) {
      const { token } = await createToken(db, {
        regionKey: 'eu',
        ttlSeconds: 600,
        createdBy: 'e2e',
      })
      const container = `blockly-${DEPLOYMENT}-${name}`
      docker(
        'run',
        '-d',
        '--privileged',
        '--name',
        container,
        '--hostname',
        `node-${name}`,
        '--network',
        NETWORK,
        '--ip',
        ip,
        '-v',
        `${BIN}:/usr/local/bin/pryxd:ro`,
        '-e',
        'DOCKER_TLS_CERTDIR=',
        DIND,
      )
      const boot = () => {
        docker(
          'exec',
          '-d',
          container,
          'sh',
          '-c',
          'pryxd serve --config /etc/pryxd/pryxd.toml --log-format pretty >> /var/log/pryxd.log 2>&1',
        )
      }
      await until(
        `${name}'s Docker`,
        async () => spawnSync('docker', ['exec', container, 'docker', 'info']).status === 0,
      )
      spawnSync('sh', ['-c', `docker exec -i ${container} docker load < ${join(dir, 'image.tar')}`])
      // No systemd in the container: join writes everything, and the test starts serve itself.
      console.warn(
        docker(
          'exec',
          container,
          'pryxd',
          'join',
          joinToken(joinPoint, token),
          '--allow-image',
          'alpine:',
          '--no-start',
        ),
      )
      const configured =
        docker('exec', container, 'cat', '/etc/pryxd/pryxd.toml').replace(
          '[workloads]',
          '[workloads]\nmin_memory_mb = 64',
        ) + POLICY
      Bun.TOML.parse(configured)
      const file = join(dir, `${name}.toml`)
      writeFileSync(file, configured, { mode: 0o600 })
      docker('cp', file, `${container}:/etc/pryxd/pryxd.toml`)
      boot()
      node[name] = { name, container, ip, id: '', boot }
    }
    const enrolled = await until('both nodes to enroll and beat', async () => {
      const summaries = await nodeSummaries(db, THRESHOLDS)
      return summaries.length === 2 && summaries.every((s) => s.derivedHealth === 'healthy')
        ? summaries
        : null
    }).catch((error) => {
      for (const item of Object.values(node))
        console.error(docker('exec', item.container, 'cat', '/var/log/pryxd.log'))
      throw error
    })
    for (const summary of enrolled) {
      const which = summary.edge_host === node.a.ip ? 'a' : 'b'
      node[which].id = summary.id
    }
    await runtime.refreshRegistry()
  }, 300_000)

  afterAll(async () => {
    for (const n of Object.values(node)) spawnSync('docker', ['rm', '-f', '-v', n.container])
    if (env.S3_TEST_CONTAINER) spawnSync('docker', ['network', 'disconnect', NETWORK, env.S3_TEST_CONTAINER])
    spawnSync('docker', ['network', 'rm', NETWORK])
    nodes?.close()
    endpoint?.kill('SIGTERM')
    await endpoint?.exited
    await drop?.()
    if (env.PRYXD_E2E_KEEP === '1') console.warn(`kept ${dir}`)
    else spawnSync('rm', ['-rf', dir])
  })

  test('two nodes enroll by their own names, at their own addresses', () => {
    expect(node.a.id).not.toBe('')
    expect(node.b.id).not.toBe('')
    expect(node.a.id).not.toBe(node.b.id)
  })

  test('the line an operator pastes checks the CA, then fetches join.sh and pryxd over it', () => {
    const run = spawnSync('sh', ['-c', fetchedAsPasted(joinPoint)])
    expect(run.status, run.stderr.toString()).toBe(0)
    // A token naming another CA stops the line before anything is fetched over the one served.
    const stopped = spawnSync('sh', ['-c', joinCommand({ ...joinPoint, caPem: 'another CA' }, 'bk1.e30')])
    expect(stopped.status).not.toBe(0)
    expect(stopped.stdout.toString() + stopped.stderr.toString()).toContain('FAILED')
  })

  test(
    'a server is placed on one of them, and a snapshot of it reaches the store',
    async () => {
      h1 = await runtime.ensureProvisioned(s1, { regionKey: 'eu' }, SPEC, progress)
      await runtime.waitRunning(h1, AbortSignal.timeout(MINUTE_MS))
      expect([node.a.id, node.b.id]).toContain(nodeOf(h1) ?? '')
      await exec(h1, 'echo before-move > /data/data/level.dat')
      lastSnapshot = (await runtime.snapshot(h1)).snapshot
      await uploaded()
    },
    2 * MINUTE_MS,
  )

  test(
    'an operator moves it to the other node, and its data goes along without its jar',
    async () => {
      const from = nodeOf(h1)
      const to = other(from)
      await runtime.requestMove(s1, to.id, 'e2e')
      // What the application does around a move: stop, relocate, start where it landed.
      await runtime.stop(h1)
      const moved = await runtime.relocate(h1, { regionKey: 'eu' }, SPEC, progress)
      expect(nodeOf(moved)).toBe(to.id)
      expect(decodeHandle(moved).epoch).toBeGreaterThan(decodeHandle(h1).epoch)
      expect(runtime.endpoint(moved, 'game', 'edge').host).toBe(to.ip)
      h1 = moved
      await runtime.start(h1)
      await runtime.waitRunning(h1, AbortSignal.timeout(MINUTE_MS))
      expect(await exec(h1, 'cat /data/data/level.dat')).toBe('before-move')
      // The source's copy is fenced and gone; the move's archive with it.
      const source = from === node.a.id ? node.a : node.b
      await until('the source copy to go', async () => {
        await runtime.tidy()
        return !inner(source, 'ps', '-a', '--format', '{{.Names}}').includes(s1)
      })
      const moves = await rows<{ status: string }>(
        db,
        sql`SELECT status FROM fleet_archives WHERE purpose = 'move'`,
      )
      expect(moves.map((m) => m.status)).toEqual(['deleted'])
    },
    3 * MINUTE_MS,
  )

  test(
    'a drained node takes nothing new',
    async () => {
      const busy = nodeOf(h1) ?? ''
      await drain(db, busy, 'e2e')
      const h2 = await runtime.ensureProvisioned(s2, { regionKey: 'eu' }, SPEC, progress)
      expect(nodeOf(h2)).toBe(other(busy).id)
      await undrain(db, busy, 'e2e')
      await runtime.destroy(h2)
    },
    2 * MINUTE_MS,
  )

  test(
    'a sleeping server whose node is full moves to one with room, and starts there',
    async () => {
      const home = nodeOf(h1) === node.a.id ? node.a : node.b
      const away = other(home.id)
      const ledger = async () => new Map((await nodeSummaries(db, THRESHOLDS)).map((n) => [n.id, n]))
      await exec(h1, 'echo slept > /data/data/level.dat')
      // Asleep, its data stays on its node and its memory doesn't, as soon as the stop is answered.
      await runtime.stop(h1)
      expect((await ledger()).get(home.id)?.running.memoryMb).toBe(0)
      // Another server, there, takes all but 64 MB of the node's 2048 to run.
      await drain(db, away.id, 'e2e')
      const h3 = await runtime.ensureProvisioned(
        s3,
        { regionKey: 'eu' },
        { ...SPEC, resources: { memoryMb: 1984 } },
        progress,
      )
      await undrain(db, away.id, 'e2e')
      expect(nodeOf(h3)).toBe(home.id)
      await runtime.waitRunning(h3, AbortSignal.timeout(MINUTE_MS))
      // A start without its spec can't move it: refused, and nothing claimed for it.
      await expect(runtime.start(h1)).rejects.toBeInstanceOf(RuntimeFull)
      expect((await ledger()).get(home.id)?.running.memoryMb).toBe(1984)
      // A start with the server's spec, as the application's is, moves it first.
      const woken = await runtime.ensureProvisioned(s1, { regionKey: 'eu' }, SPEC, progress)
      expect(nodeOf(woken)).toBe(away.id)
      expect(decodeHandle(woken).epoch).toBeGreaterThan(decodeHandle(h1).epoch)
      h1 = woken
      await runtime.waitRunning(h1, AbortSignal.timeout(MINUTE_MS))
      expect(await exec(h1, 'cat /data/data/level.dat')).toBe('slept')
      const moved = await rows<{
        node_id: string
        data: { fromNode: string; why: string }
      }>(db, sql`SELECT node_id, data FROM fleet_events WHERE kind = 'placement.moved_for_room'`)
      expect(moved.map((e) => [e.data.fromNode, e.node_id])).toEqual([[home.id, away.id]])
      expect(moved[0]?.data.why).toBe('memory: 64 MB free beside the sandboxes running there, needs 128')
      const held = await ledger()
      expect(held.get(home.id)?.running.memoryMb).toBe(1984)
      expect(held.get(home.id)?.allocated.memoryMb).toBe(1984)
      expect(held.get(away.id)?.running.memoryMb).toBe(128)
      // A server that stops on its own gives its memory back once its node reports it stopped.
      await runtime.exec(h3, ['sh', '-c', 'kill 1'], 10).catch(() => undefined)
      await until('the stopped server to hold no memory', async () => {
        return (await ledger()).get(home.id)?.running.memoryMb === 0
      })
      await runtime.destroy(h3)
    },
    4 * MINUTE_MS,
  )

  for (const scenario of ANSWERS)
    test(scenario.name, () => scenario.run({ db, runtime, node, h1 }), scenario.ms)

  test(
    'a host lost is rebuilt elsewhere from its last snapshot, never from what it said later',
    async () => {
      await exec(h1, 'echo snapshotted > /data/data/level.dat')
      lastSnapshot = (await runtime.snapshot(h1)).snapshot
      await uploaded()
      await exec(h1, 'echo after-the-snapshot > /data/data/level.dat')
      const lost = nodeOf(h1) === node.a.id ? node.a : node.b
      docker('kill', lost.container)
      await until(
        'the node to read unavailable',
        async () => (await runtime.observe(h1)).state === 'unknown',
        30_000,
      )
      // Unavailable isn't lost: only an operator who knows the host is down says so.
      expect((await runtime.observe(h1)).hostLost).toBeUndefined()
      await confirmLost(db, THRESHOLDS, lost.id, {
        fencedBy: 'docker kill',
        reason: 'e2e',
        by: 'e2e',
      })
      expect((await runtime.observe(h1)).hostLost).toBe(true)
      const rebuilt = await runtime.relocate(h1, { regionKey: 'eu' }, SPEC, progress, lastSnapshot)
      expect(nodeOf(rebuilt)).toBe(other(lost.id).id)
      h1 = rebuilt
      await runtime.start(h1)
      await runtime.waitRunning(h1, AbortSignal.timeout(MINUTE_MS))
      expect(await exec(h1, 'cat /data/data/level.dat')).toBe('snapshotted')
    },
    3 * MINUTE_MS,
  )

  test(
    'the lost node comes back, its old copy is fenced and never runs, and the fork is reported',
    async () => {
      const lost = nodeOf(h1) === node.a.id ? node.b : node.a
      docker('start', lost.container)
      await until(
        `${lost.name}'s Docker`,
        async () => spawnSync('docker', ['exec', lost.container, 'docker', 'info']).status === 0,
      )
      lost.boot()
      await until('the fork to be reported', async () => {
        const found = await rows(db, sql`SELECT 1 FROM fleet_events WHERE kind = 'fork.detected'`)
        return found.length > 0
      })
      const events = await rows<{ kind: string }>(
        db,
        sql`SELECT kind FROM fleet_events WHERE node_id = ${lost.id} ORDER BY id`,
      )
      expect(events.map((e) => e.kind)).toContain('node.returned_while_lost')
      await sleep(3 * SECOND_MS)
      const states = inner(lost, 'ps', '-a', '--format', '{{.Names}} {{.State}}')
      expect(states).toContain(s1)
      expect(states).not.toMatch(new RegExp(`${s1} running`))
      // The rebuilt server is the one that runs.
      expect((await runtime.observe(h1)).state).toBe('running')
    },
    3 * MINUTE_MS,
  )
})
