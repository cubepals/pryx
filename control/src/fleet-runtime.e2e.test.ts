// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

import { afterAll, beforeAll, describe, expect, test } from 'bun:test'
import { spawnSync } from 'node:child_process'
import { createHash, generateKeyPairSync, randomBytes, randomUUID, sign } from 'node:crypto'
import {
  chmodSync,
  copyFileSync,
  existsSync,
  mkdirSync,
  openSync,
  readFileSync,
  writeFileSync,
} from 'node:fs'
import { createServer } from 'node:net'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { MINUTE_MS } from '@blockly/contracts/durations'
import type { Db } from '@blockly/db'
import { sql } from 'drizzle-orm'
import type { ProgressSink, RuntimeHandle, RuntimeSpec } from '../../app/ports/runtime.ts'
import { runtimeKey } from '../../app/ports/runtime.ts'
import { freshDatabase, hasDatabase } from '../../testing/fleet.ts'
import { S3ArchiveStore } from '../s3/s3-archive-store.ts'
import { createTestBucket } from '../s3/test-bucket.ts'
import { controlPlaneName, FleetCa } from './ca.ts'
import { FleetRuntime } from './fleet-runtime.ts'
import { decodeHandle } from './handle.ts'
import { joinToken } from './join-token.ts'
import { NodeClient } from './node-client.ts'
import { createOperatorApi } from './operator-api.ts'
import { DEFAULTS } from './placement.ts'
import { confirmLost, createToken, nodeSummaries, reinstate } from './registry.ts'
import { rows } from './sql.ts'
import { FleetStore } from './store.ts'

/**
 * The production adapter against a real blocklyd: its binary, enrolled over the node endpoint's
 * mutual TLS, running alpine workloads on this machine's Docker, snapshots and archives through a
 * real S3 store. One node, which is a whole deployment (docs/fleet.md, "One machine"). Then another
 * node upgrades itself to the blocklyd the node endpoint serves, under a stand-in for systemd, and
 * rolls back from one that can't start (docs/fleet.md, "Upgrades").
 *
 *   git clone -b v$(bun scripts/blocklyd.ts version) https://github.com/cubepals/blocklyd ../blocklyd
 *   cargo build --manifest-path ../blocklyd/Cargo.toml     # debug: only it takes BLOCKLYD_TEST_VERSION
 *   BLOCKLYD_BIN=../blocklyd/target/debug/blocklyd DATABASE_URL=… S3_TEST_ENDPOINT=… bun test fleet-runtime.e2e
 *
 * blocklyd runs as root (it hands data directories to the workload's user); BLOCKLYD_SUDO=1 runs it
 * through `sudo -n` where the tests themselves don't run as root.
 */

const env = process.env
const BIN = env.BLOCKLYD_BIN
const SUDO = env.BLOCKLYD_SUDO === '1'
const S3 = env.S3_TEST_ENDPOINT
const ready = Boolean(BIN && existsSync(BIN) && S3 && hasDatabase && existsSync('/var/run/docker.sock'))

const DEPLOYMENT = `e2e${randomBytes(3).toString('hex')}`
const THRESHOLDS = { suspectSeconds: 3, unavailableSeconds: 6 }
const credentials = {
  accessKeyId: env.S3_TEST_ACCESS_KEY_ID ?? 'blockly-test',
  secretAccessKey: env.S3_TEST_SECRET_ACCESS_KEY ?? 'blockly-test-secret',
}

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

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms))

async function until<T>(
  what: string,
  check: () => Promise<T | null | undefined | false>,
  ms = 30_000,
): Promise<T> {
  const deadline = Date.now() + ms
  for (;;) {
    const value = await check().catch(() => null)
    if (value) return value
    if (Date.now() > deadline) throw new Error(`Timed out waiting for ${what}`)
    await sleep(200)
  }
}

const progress: ProgressSink = { step: async () => {}, handle: async () => {} }

// A server stand-in: makes its world on first start, keeps it, and stops when asked.
const SCRIPT = [
  'mkdir -p /data/world /data/libraries',
  '[ -f /data/world/level.dat ] || echo fresh > /data/world/level.dat',
  'echo jar > /data/libraries/server.jar',
  'echo started',
  "trap 'echo stopping; exit 0' TERM",
  'while true; do sleep 1; done',
].join('; ')

const SPEC: RuntimeSpec = {
  image: 'alpine:3.22',
  entrypoint: ['sh', '-c', SCRIPT],
  env: { EULA: 'TRUE' },
  secrets: { RCON_PASSWORD: 'e2e-secret' },
  resources: { memoryMb: 128 },
  storage: { mountPath: '/data', sizeGb: 1, reconstructible: ['libraries'] },
  ports: [{ name: 'game', port: 25565, protocol: 'tcp', audience: ['edge', 'control'] }],
  stop: { signal: 'SIGTERM', timeoutSeconds: 5 },
  labels: { 'blockly.server': 'e2e' },
}

interface Daemon {
  config: string
  log: string
  opsPort: number
  start: () => void
  kill: (signal: 'KILL' | 'TERM') => void
  /** `blocklyd join` on this node, with a stand-in for systemctl that stops and starts it. */
  join: (token: string) => ReturnType<typeof spawnSync>
}

/** blocklyd serving `config` as a process of its own, as root, with its log in `home`. */
function daemon(home: string, config: string, opsPort: number): Daemon {
  const log = join(home, 'blocklyd.log')
  return {
    config,
    log,
    opsPort,
    start: () => {
      const out = openSync(log, 'a')
      const command = [BIN ?? '', 'serve', '--config', config, '--log-format', 'pretty']
      Bun.spawn(SUDO ? ['sudo', '-n', ...command] : command, { stdout: out, stderr: out })
    },
    kill: (signal) => {
      const pkill = ['pkill', `-${signal}`, '-f', '--', `--config ${config}`]
      spawnSync(SUDO ? 'sudo' : (pkill[0] ?? 'pkill'), SUDO ? ['-n', ...pkill] : pkill.slice(1))
    },
    join: (token) => {
      // `serve --config`, so the pattern never matches the join that runs this.
      const serving = `serve --config ${config}`
      const systemctl = join(home, 'systemctl')
      writeFileSync(
        systemctl,
        [
          '#!/bin/sh',
          'case "$1" in',
          `stop) pkill -TERM -f -- '${serving}'; while pgrep -f -- '${serving}' >/dev/null; do sleep 0.2; done ;;`,
          `start|restart) nohup '${BIN}' ${serving} --log-format pretty >>'${log}' 2>&1 & ;;`,
          'esac',
          '',
        ].join('\n'),
        { mode: 0o755 },
      )
      const command = [BIN ?? '', 'join', token, '--config', config, '--systemctl', systemctl]
      return spawnSync(SUDO ? 'sudo' : (command[0] ?? ''), SUDO ? ['-n', ...command] : command.slice(1), {
        encoding: 'utf8',
        timeout: 180_000,
      })
    },
  }
}

/** The node endpoint at `url`, on Node as production runs it (endpoint.e2e.ts says why), with this CA. */
async function startEndpoint(cert: string, key: string, url: string, database: string) {
  const args = [cert, key, new URL(url).port, DEPLOYMENT]
  const endpoint = Bun.spawn(['node', join(import.meta.dir, 'endpoint.e2e.ts'), ...args], {
    env: { ...env, DATABASE_URL: database },
    stdout: 'pipe',
    stderr: 'inherit',
  })
  const first = await (endpoint.stdout as ReadableStream<Uint8Array>).getReader().read()
  if (!new TextDecoder().decode(first.value).includes('listening'))
    throw new Error('the node endpoint failed')
  return endpoint
}

/** The test bucket, as the fleet's own store and as the application's archive. */
const BUCKET = `blockly-${DEPLOYMENT}`
const archiveStore = () =>
  new S3ArchiveStore({
    endpoint: S3 ?? '',
    runtimeEndpoint: null,
    bucket: BUCKET,
    region: 'auto',
    ...credentials,
  })

/** A node's blocklyd.toml: its own state, API and port range, enrolling with the token in `home`. */
function nodeConfig(at: {
  home: string
  endpointUrl: string
  dir: string
  apiPort: number
  opsPort: number
  low: number
}): string {
  const { home, endpointUrl, dir, apiPort, opsPort, low } = at
  return `deployment_id = "${DEPLOYMENT}"
state_dir = "${join(home, 'state')}"
[api]
listen = "127.0.0.1:${apiPort}"
[ops]
listen = "127.0.0.1:${opsPort}"
[network]
port_range = [${low}, ${low + 49}]
port_quarantine_seconds = 0
[workloads]
allowed_images = ["alpine:"]
min_memory_mb = 64
[capacity]
reserved_memory_mb = 0
allocatable_memory_mb = 4096
min_free_disk_mb = 256
[transfer]
max_put_mb = 1
[fleet]
url = "${endpointUrl}"
ca = "${join(dir, 'ca.pem')}"
enrollment_token_file = "${join(home, 'token')}"
heartbeat_seconds = 1
`
}

describe.skipIf(!ready)('FleetRuntime against a real blocklyd', () => {
  const dir = join(tmpdir(), `blockly-${DEPLOYMENT}`)
  let db: Db
  let drop: () => Promise<void>
  let endpoint: ReturnType<typeof Bun.spawn> | undefined
  let endpointUrl: string
  let database: string
  let runtime: FleetRuntime
  let nodes: NodeClient
  let node: Daemon
  let handle: RuntimeHandle
  const key = runtimeKey('0b6f1f2e-6a47-4c9a-9a39-2f4ac7e51f10')

  const exec = async (command: string) =>
    (await runtime.exec(handle, ['sh', '-c', command], 10)).stdout.trim()
  const containerStarted = () =>
    spawnSync('docker', [
      'inspect',
      '-f',
      '{{.State.StartedAt}}',
      `blockly-${DEPLOYMENT}-${key}`,
    ]).stdout.toString()

  beforeAll(async () => {
    mkdirSync(dir, { recursive: true })
    const fresh = await freshDatabase()
    ;({ db, drop } = fresh)
    database = fresh.url
    await createTestBucket(S3 ?? '', credentials, BUCKET)

    const pems = await FleetCa.generate(DEPLOYMENT)
    const ca = await FleetCa.fromPem(pems.certPem, pems.keyPem)
    writeFileSync(join(dir, 'ca.pem'), ca.pem)
    writeFileSync(join(dir, 'ca.key'), pems.keyPem, { mode: 0o600 })
    const endpointPort = await freePort()
    endpointUrl = `https://127.0.0.1:${endpointPort}`
    endpoint = await startEndpoint(join(dir, 'ca.pem'), join(dir, 'ca.key'), endpointUrl, database)
    const client = await ca.identity(controlPlaneName(DEPLOYMENT), 'client', 1)
    nodes = new NodeClient({ caPem: ca.pem, certPem: client.certPem, keyPem: client.keyPem }, DEPLOYMENT)
    runtime = new FleetRuntime({
      db,
      nodes,
      store: new FleetStore(archiveStore(), DEPLOYMENT),
      deployment: DEPLOYMENT,
      regionMap: {},
      placement: DEFAULTS,
      cpuMillisPerGb: 250,
      thresholds: THRESHOLDS,
      suspend: true,
      log: () => {},
    })

    // One node: its own state, API and port range, enrolling with a one-time token.
    const { token } = await createToken(db, {
      regionKey: 'eu',
      ttlSeconds: 600,
      createdBy: 'e2e',
      labels: { monthly_cost_cents: '4000' },
    })
    const home = join(dir, 'node')
    mkdirSync(home, { recursive: true })
    writeFileSync(join(home, 'token'), token)
    const apiPort = await freePort()
    const opsPort = await freePort()
    const low = 45_000 + (Number.parseInt(DEPLOYMENT.slice(3), 16) % 90) * 50
    const config = join(home, 'blocklyd.toml')
    writeFileSync(config, nodeConfig({ home, endpointUrl, dir, apiPort, opsPort, low }))
    node = daemon(home, config, opsPort)
    node.start()
    await until('the node to enroll and beat', async () => {
      const [summary] = await nodeSummaries(db, THRESHOLDS)
      return summary?.derivedHealth === 'healthy' ? summary : null
    })
    await runtime.refreshRegistry()
  }, 120_000)

  afterAll(async () => {
    node?.kill('KILL')
    const ids = spawnSync('docker', ['ps', '-aq', '--filter', `label=blocklyd.deployment=${DEPLOYMENT}`])
      .stdout.toString()
      .split(/\s+/)
      .filter(Boolean)
    if (ids.length > 0) spawnSync('docker', ['rm', '-f', ...ids])
    nodes?.close()
    endpoint?.kill('SIGTERM')
    await endpoint?.exited
    await drop?.()
    // BLOCKLYD_E2E_KEEP=1 keeps the node's state and log for a look afterwards.
    if (env.BLOCKLYD_E2E_KEEP === '1') console.warn(`kept ${dir}`)
    else if (SUDO) spawnSync('sudo', ['-n', 'rm', '-rf', dir])
    else spawnSync('rm', ['-rf', dir])
  })

  test('enrollment pinned the node, without its token on disk', async () => {
    const [summary] = await nodeSummaries(db, THRESHOLDS)
    expect(summary?.lifecycle).toBe('active')
    expect(summary?.region_key).toBe('eu')
    expect(existsSync(join(dir, 'node', 'token'))).toBe(false)
  })

  test('a server is placed, runs, and answers', async () => {
    handle = await runtime.ensureProvisioned(key, { regionKey: 'eu' }, SPEC, progress)
    await runtime.waitRunning(handle, AbortSignal.timeout(MINUTE_MS))
    expect(await exec('cat /data/world/level.dat')).toBe('fresh')
    const ref = decodeHandle(handle)
    expect(ref.epoch).toBe(1)
    const endpoint = runtime.endpoint(handle, 'game', 'edge')
    expect(endpoint.host).toBe('127.0.0.1')
    expect(endpoint.port).toBeGreaterThan(44_999)
    await until('a running observation', async () => (await runtime.observe(handle)).state === 'running')
    // Calling it again converges on what is there.
    const again = await runtime.ensureProvisioned(key, { regionKey: 'eu' }, SPEC, progress)
    expect(runtime.sameCompute(again, handle)).toBe(true)
    expect(runtime.prices(handle, { memoryMb: 128, storageGb: 1 })?.storageMonthCents).toBeGreaterThan(0)
  }, 120_000)

  test('asleep, it is suspended and wakes where it was; a change made meanwhile lands at its next boot', async () => {
    const asleep = { ...SPEC, resumesOn: 'e2e' }
    handle = await runtime.apply(handle, asleep)
    const started = containerStarted()
    expect(await runtime.sleep(handle)).toBe('suspended')
    const reported = async () =>
      (
        await rows<{ state: string }>(db, sql`SELECT state FROM fleet_observations WHERE workload = ${key}`)
      )[0]
    await until('a suspended report', async () => (await reported())?.state === 'suspended')
    const changed = { ...asleep, env: { ...SPEC.env, MOTD: 'Back soon' } }
    const steps: string[] = []
    const sink: ProgressSink = { step: async (step) => void steps.push(step), handle: async () => {} }
    expect(await runtime.ensureProvisioned(key, { regionKey: 'eu' }, changed, sink)).toBe(handle)
    await runtime.waitRunning(handle, AbortSignal.timeout(MINUTE_MS))
    expect(steps).toEqual(['allocating', 'compute', 'resuming'])
    expect(containerStarted()).toBe(started)
    expect(await exec('echo "[$MOTD]"')).toBe('[]')
    await runtime.stop(handle)
    await runtime.ensureProvisioned(key, { regionKey: 'eu' }, changed, progress)
    await runtime.waitRunning(handle, AbortSignal.timeout(MINUTE_MS))
    expect(containerStarted()).not.toBe(started)
    expect(await exec('echo "[$MOTD]"')).toBe('[Back soon]')
  }, 120_000)

  test('a snapshot restores in place, from the node, and reaches the store', async () => {
    await exec('echo day-one > /data/world/level.dat')
    const taken = await runtime.snapshot(handle)
    expect(taken.sizeBytes).toBeGreaterThan(0)
    await until('the upload', async () => {
      await runtime.uploadPending()
      const [row] = await rows<{ status: string }>(db, sql`SELECT status FROM fleet_archives`)
      return row?.status === 'ready'
    })
    await exec('echo griefed > /data/world/level.dat')
    await runtime.stop(handle)
    handle = await runtime.restore(handle, { kind: 'snapshot', snapshot: taken.snapshot }, SPEC, progress)
    expect(decodeHandle(handle).epoch).toBe(2)
    await runtime.start(handle)
    await runtime.waitRunning(handle, AbortSignal.timeout(MINUTE_MS))
    expect(await exec('cat /data/world/level.dat')).toBe('day-one')
    expect(await runtime.goneSnapshots([taken.snapshot])).toEqual(new Set())
  }, 120_000)

  test('released, the world rests in the store and comes back from it', async () => {
    await exec('echo resting > /data/world/level.dat')
    await runtime.stop(handle)
    const taken = await runtime.snapshot(handle)
    await until('the upload', async () => {
      await runtime.uploadPending()
      const [row] = await rows<{ status: string }>(
        db,
        sql`SELECT status FROM fleet_archives ORDER BY captured_at DESC LIMIT 1`,
      )
      return row?.status === 'ready'
    })
    // The archive the application keeps, as it would export one for a stored world.
    const archives = archiveStore()
    const keyName = `stored/${DEPLOYMENT}.tar.gz`
    const put = await archives.archiveTarget(keyName, 600, 'runtime')
    const exported = await runtime.exportSnapshot(taken.snapshot, put)
    expect(exported.sha256).toMatch(/^[0-9a-f]{64}$/)
    const released = await runtime.release(handle)
    expect(decodeHandle(released).node).toBeNull()
    expect(spawnSync('docker', ['inspect', `blockly-${DEPLOYMENT}-${key}`]).status).not.toBe(0)
    const download = await archives.presignGet(keyName, 600, 'runtime')
    handle = await runtime.restore(released, { kind: 'archive', download }, SPEC, progress)
    await runtime.start(handle)
    await runtime.waitRunning(handle, AbortSignal.timeout(MINUTE_MS))
    expect(await exec('cat /data/world/level.dat')).toBe('resting')
  }, 180_000)

  test('a world larger than one PUT the node makes goes in parts, and comes back whole', async () => {
    // 12 MiB that doesn't compress, from a node that puts no more than 1 MiB in one PUT: the
    // store's limit, as R2's is 5 GiB, made small.
    const written = await exec(
      'head -c 12582912 /dev/urandom > /data/world/region.mca && sha256sum /data/world/region.mca',
    )
    const taken = await runtime.snapshot(handle)
    await until('the upload', async () => {
      await runtime.uploadPending()
      const [row] = await rows<{ status: string }>(
        db,
        sql`SELECT status FROM fleet_archives ORDER BY captured_at DESC LIMIT 1`,
      )
      return row?.status === 'ready'
    })
    const [copy] = await rows<{ object_key: string; size_bytes: string; sha256: string }>(
      db,
      sql`SELECT object_key, size_bytes, sha256 FROM fleet_archives ORDER BY captured_at DESC LIMIT 1`,
    )
    const archives = archiveStore()
    const sha256 = async (key: string) => {
      const read = await fetch((await archives.presignGet(key, 600, 'browser')).url)
      return createHash('sha256')
        .update(Buffer.from(await read.arrayBuffer()))
        .digest('hex')
    }
    // The fleet's own copy went in parts: whole, and the bytes the node hashed.
    expect(Number(copy?.size_bytes)).toBeGreaterThan(12 * 1024 ** 2)
    expect(await archives.head(copy?.object_key ?? '')).toEqual({ sizeBytes: Number(copy?.size_bytes) })
    expect(await sha256(copy?.object_key ?? '')).toBe(copy?.sha256 ?? '')
    // So does an export to the application's archive, and the world comes back from it.
    const keyName = `large/${DEPLOYMENT}.tar.gz`
    const exported = await runtime.exportSnapshot(
      taken.snapshot,
      await archives.archiveTarget(keyName, 600, 'runtime'),
    )
    expect(await sha256(keyName)).toBe(exported.sha256)
    await exec('echo griefed > /data/world/region.mca')
    await runtime.stop(handle)
    const download = await archives.presignGet(keyName, 600, 'runtime')
    handle = await runtime.restore(handle, { kind: 'archive', download }, SPEC, progress)
    await runtime.start(handle)
    await runtime.waitRunning(handle, AbortSignal.timeout(MINUTE_MS))
    expect(await exec('sha256sum /data/world/region.mca')).toBe(written)
    await exec('rm /data/world/region.mca')
  }, 180_000)

  test('a world from another runtime is adopted, and comes from its archive onto a node', async () => {
    // What a move between runtimes asks of the fleet (docs/runtimes.md): a server it never held,
    // its world only in the archive store.
    expect(await runtime.hasRoom({ regionKey: 'eu' }, { memoryMb: 512, storageGb: 1 })).toBe(true)
    expect(await runtime.hasRoom({ regionKey: 'eu' }, { memoryMb: 10_000_000, storageGb: 1 })).toBe(false)
    const held = await runtime.capacity()
    expect(held.machines).toHaveLength(1)
    expect(held.machines[0]?.servers).toBe(1)
    await exec('echo elsewhere > /data/world/moved.txt')
    await runtime.stop(handle)
    const taken = await runtime.snapshot(handle)
    const archives = archiveStore()
    const keyName = `moved/${DEPLOYMENT}.tar.gz`
    await runtime.exportSnapshot(taken.snapshot, await archives.archiveTarget(keyName, 600, 'runtime'))
    const other = runtimeKey(randomUUID())
    const adopted = await runtime.adopt(other, { regionKey: 'eu' }, SPEC)
    expect(runtime.owns(adopted)).toBe(true)
    expect(decodeHandle(adopted).node).toBeNull()
    expect(await runtime.adopt(other, { regionKey: 'eu' }, SPEC)).toBe(adopted)
    const download = await archives.presignGet(keyName, 600, 'runtime')
    const restored = await runtime.restore(adopted, { kind: 'archive', download }, SPEC, progress)
    expect(decodeHandle(restored).node).not.toBeNull()
    await runtime.start(restored)
    await runtime.waitRunning(restored, AbortSignal.timeout(MINUTE_MS))
    const read = await runtime.exec(restored, ['cat', '/data/world/moved.txt', '/data/world/level.dat'], 30)
    expect(read.stdout.trim().split('\n')).toEqual(['elsewhere', 'resting'])
    await runtime.destroy(restored)
    await runtime.start(handle)
    await runtime.waitRunning(handle, AbortSignal.timeout(MINUTE_MS))
  }, 180_000)

  test('blocklyd dying leaves the server running, and coming back adopts it untouched', async () => {
    const started = containerStarted()
    node.kill('KILL')
    await until(
      'the node to read unavailable',
      async () => (await runtime.observe(handle)).state === 'unknown',
      20_000,
    )
    expect(containerStarted()).toBe(started)
    node.start()
    await until(
      'a running observation again',
      async () => (await runtime.observe(handle)).state === 'running',
    )
    expect(containerStarted()).toBe(started)
    expect(await exec('cat /data/world/level.dat')).toBe('resting')
  }, 120_000)

  test('a node re-enrolled with the line, through blocklyd join, keeps its id and its servers', async () => {
    const [before] = await nodeSummaries(db, THRESHOLDS)
    const started = containerStarted()
    const { token } = await createToken(db, {
      regionKey: '',
      ttlSeconds: 600,
      createdBy: 'e2e',
      node: before?.id ?? '',
    })
    const caPem = readFileSync(join(dir, 'ca.pem'), 'utf8')
    const joined = node.join(
      joinToken({ url: endpointUrl, caPem, deployment: DEPLOYMENT }, token, before?.id),
    )
    expect(`${joined.stdout}${joined.stderr}`).toContain(`Re-enrolled this host as node ${before?.id}`)
    expect(joined.status).toBe(0)
    expect(joined.stdout).not.toContain('Replaced')
    const [after] = await nodeSummaries(db, THRESHOLDS)
    expect(after?.id).toBe(before?.id ?? '')
    expect(after?.cert_sha256).not.toBe(before?.cert_sha256)
    await until(
      'the node to read healthy',
      async () => (await nodeSummaries(db, THRESHOLDS))[0]?.derivedHealth === 'healthy',
    )
    expect(await nodeSummaries(db, THRESHOLDS)).toHaveLength(1)
    await until('a running observation', async () => (await runtime.observe(handle)).state === 'running')
    expect(containerStarted()).toBe(started)
    expect(await exec('cat /data/world/level.dat')).toBe('resting')
  }, 240_000)

  test('a node held lost stops answering for its servers, and is fenced off them', async () => {
    const [summary] = await nodeSummaries(db, THRESHOLDS)
    const id = summary?.id ?? ''
    await confirmLost(db, THRESHOLDS, id, { fencedBy: 'e2e', reason: 'e2e', by: 'e2e', force: true })
    const seen = await runtime.observe(handle)
    expect(seen.hostLost).toBe(true)
    // Its lease is gone at the next beat: nothing restarts there on its own.
    await until('the lease to lapse', async () => {
      const text = await (await fetch(`http://127.0.0.1:${node.opsPort}/metrics`)).text()
      return /blocklyd_fleet_lease_remaining_seconds 0(\.0)?\n/.test(text)
    })
    await reinstate(db, id, 'e2e')
    await until(
      'a running observation once reinstated',
      async () => (await runtime.observe(handle)).state === 'running',
    )
  }, 60_000)

  test('the operator API sees the node and the server', async () => {
    const api = createOperatorApi({
      db,
      runtime,
      thresholds: THRESHOLDS,
      token: 'operator-token',
      relocate: async () => {},
    })
    const headers = { authorization: 'Bearer operator-token', 'x-operator': 'e2e' }
    const listed = await api.fetch(new Request('http://x/fleet/v1/nodes', { headers }))
    expect(listed.status).toBe(200)
    const body = (await listed.json()) as {
      nodes: Array<{
        servers: number
        memoryMb: { runningServers: number }
        disk: { reflink: boolean | null }
      }>
    }
    expect(body.nodes[0]?.servers).toBe(1)
    expect(body.nodes[0]?.memoryMb.runningServers).toBe(128)
    expect(typeof body.nodes[0]?.disk.reflink).toBe('boolean')
    const refused = await api.fetch(new Request('http://x/fleet/v1/nodes'))
    expect(refused.status).toBe(401)
  })

  test('destroyed, nothing is left', async () => {
    await runtime.destroy(handle)
    expect(spawnSync('docker', ['inspect', `blockly-${DEPLOYMENT}-${key}`]).status).not.toBe(0)
    const left = await rows<{ workload: string }>(db, sql`SELECT workload FROM fleet_placements`)
    expect(left).toEqual([])
  }, 60_000)

  // Last: the runtime here still holds a client certificate from the old CA.
  test('moved to a new fleet CA by its line, the node comes back healthy under its own id', async () => {
    const [before] = await nodeSummaries(db, THRESHOLDS)
    // Steps 1-2 of docs/fleet-operations.md §8: the control plane runs a new CA.
    const pems = await FleetCa.generate(DEPLOYMENT)
    writeFileSync(join(dir, 'ca-new.pem'), pems.certPem)
    writeFileSync(join(dir, 'ca-new.key'), pems.keyPem, { mode: 0o600 })
    endpoint?.kill('SIGTERM')
    await endpoint?.exited
    endpoint = await startEndpoint(join(dir, 'ca-new.pem'), join(dir, 'ca-new.key'), endpointUrl, database)
    // What `fleet.ts rotate-ca` prints for this node.
    const { token } = await createToken(db, {
      regionKey: '',
      ttlSeconds: 600,
      createdBy: 'e2e',
      node: before?.id ?? '',
    })
    const point = { url: endpointUrl, caPem: pems.certPem, deployment: DEPLOYMENT }
    const joined = node.join(joinToken(point, token, before?.id))
    expect(`${joined.stdout}${joined.stderr}`).toContain(`Re-enrolled this host as node ${before?.id}`)
    expect(joined.status).toBe(0)
    expect(joined.stdout).toContain('Replaced')
    expect(readFileSync(join(dir, 'ca.pem'), 'utf8')).toBe(pems.certPem)
    await until('the node to read healthy', async () => {
      const [after] = await nodeSummaries(db, THRESHOLDS)
      return after?.id === before?.id && after?.derivedHealth === 'healthy'
    })
  }, 240_000)
})

/**
 * systemd's part, as the unit sets it: blocklyd run from the state directory after its
 * ExecStartPre, and after each stop the unit's ExecStopPost, then a start again. ExecStopPost is
 * told what systemd tells it: SERVICE_RESULT always, EXIT_CODE and EXIT_STATUS only once the main
 * process ran. Its first run reports 0.0.1 (a debug build's BLOCKLYD_TEST_VERSION), so the node is
 * older than the blocklyd it is offered. `$2` is the test's release key, which a debug build takes
 * as BLOCKLYD_TEST_RELEASE_KEY besides the keys compiled into it.
 */
const SUPERVISOR = [
  'v=0.0.1',
  'while :; do',
  '  if "$0" doctor --preflight --config "$1"; then',
  '    BLOCKLYD_TEST_RELEASE_KEY="$2" BLOCKLYD_TEST_VERSION=$v "$0" serve --config "$1" --log-format pretty; s=$?; v=',
  '    r=success; [ $s = 0 ] || r=exit-code',
  '    SERVICE_RESULT=$r EXIT_CODE=exited EXIT_STATUS=$s "$0.prev" upgrade --exited --config "$1"',
  '  else',
  '    SERVICE_RESULT=exit-code "$0.prev" upgrade --exited --config "$1"',
  '  fi',
  '  sleep 1',
  'done',
].join('\n')

/** A command as root: through `sudo -n` where the tests don't run as root. */
const asRoot = (command: string[]) =>
  spawnSync(SUDO ? 'sudo' : (command[0] ?? ''), SUDO ? ['-n', ...command] : command.slice(1))

/** The node endpoint in a process of its own (endpoint.e2e.ts), handing out `bin`. */
async function startServingEndpoint(args: string[], databaseUrl: string, bin: string) {
  const endpoint = Bun.spawn(['node', join(import.meta.dir, 'endpoint.e2e.ts'), ...args], {
    env: { ...env, DATABASE_URL: databaseUrl, BLOCKLYD_STATIC_BIN: bin },
    stdout: 'pipe',
    stderr: 'inherit',
  })
  const first = await (endpoint.stdout as ReadableStream<Uint8Array>).getReader().read()
  if (!new TextDecoder().decode(first.value).includes('listening'))
    throw new Error('the node endpoint failed')
  return endpoint
}

/** The first fleet event of `kind`, once there is one. */
const firstEvent = (db: Db, kind: string) =>
  until(
    kind,
    async () =>
      (
        await rows<{ data: { version: string; from?: string; reason?: string } }>(
          db,
          sql`SELECT data FROM fleet_events WHERE kind = ${kind} ORDER BY id`,
        )
      )[0],
    90_000,
  )

describe.skipIf(!ready)('blocklyd upgrading itself to the blocklyd the node endpoint serves', () => {
  const dir = join(tmpdir(), `blockly-${DEPLOYMENT}-upgrade`)
  const config = join(dir, 'blocklyd.toml')
  const bin = join(dir, 'state', 'bin', 'blocklyd')
  // The endpoint serves a copy, so the release's signature can go beside it.
  const served = join(dir, 'served', 'blocklyd')
  const releaseKey = join(dir, 'release-key.pub')
  const { publicKey, privateKey } = generateKeyPairSync('ed25519')
  /**
   * `blocklyd.sig` beside `path`, as blocklyd's release workflow signs a release: its version and
   * sha256, here with this test's own key, which the node is given.
   */
  const signed = (path: string) => {
    const said = spawnSync(path, ['--version']).stdout.toString().trim().split(' ')[1] ?? ''
    const sha256 = createHash('sha256').update(readFileSync(path)).digest('hex')
    writeFileSync(
      `${path}.sig`,
      sign(null, Buffer.from(`blocklyd release\n${said}\n${sha256}\n`), privateKey),
    )
    return path
  }
  let version = ''
  let db: Db
  let drop: () => Promise<void>
  let args: string[]
  let url: string
  let endpoint: ReturnType<typeof Bun.spawn> | undefined
  let supervisor: ReturnType<typeof Bun.spawn> | undefined
  const sha = (path: string) => asRoot(['sha256sum', path]).stdout.toString().split(' ')[0]
  const healthyOn = (wanted: string) =>
    until(`the node healthy on ${wanted}`, async () => {
      const [summary] = await nodeSummaries(db, THRESHOLDS)
      return summary?.derivedHealth === 'healthy' && summary.daemon_version === wanted
    })

  beforeAll(async () => {
    version =
      spawnSync(BIN ?? '', ['--version'])
        .stdout.toString()
        .trim()
        .split(' ')[1] ?? ''
    mkdirSync(join(dir, 'served'), { recursive: true })
    copyFileSync(BIN ?? '', served)
    chmodSync(served, 0o755)
    writeFileSync(releaseKey, publicKey.export({ type: 'spki', format: 'pem' }))
    const fresh = await freshDatabase()
    ;({ db, drop, url } = fresh)
    const pems = await FleetCa.generate(DEPLOYMENT)
    writeFileSync(join(dir, 'ca.pem'), pems.certPem)
    writeFileSync(join(dir, 'ca.key'), pems.keyPem, { mode: 0o600 })
    const port = await freePort()
    args = [join(dir, 'ca.pem'), join(dir, 'ca.key'), String(port), DEPLOYMENT]
    endpoint = await startServingEndpoint(args, url, signed(served))
    const { token } = await createToken(db, { regionKey: 'eu', ttlSeconds: 600, createdBy: 'e2e' })
    writeFileSync(join(dir, 'token'), token)
    const low = 47_500 + (Number.parseInt(DEPLOYMENT.slice(3), 16) % 90) * 20
    writeFileSync(
      config,
      `deployment_id = "${DEPLOYMENT}"\nstate_dir = "${join(dir, 'state')}"\n` +
        `[api]\nlisten = "127.0.0.1:${await freePort()}"\n[ops]\nlisten = "127.0.0.1:${await freePort()}"\n` +
        `[network]\nport_range = [${low}, ${low + 9}]\n[workloads]\nallowed_images = ["alpine:"]\n` +
        `[capacity]\nreserved_memory_mb = 0\nallocatable_memory_mb = 1024\nmin_free_disk_mb = 256\n` +
        `[fleet]\nurl = "https://127.0.0.1:${port}"\nca = "${join(dir, 'ca.pem')}"\n` +
        `enrollment_token_file = "${join(dir, 'token')}"\nheartbeat_seconds = 1\n`,
    )
    // Installed where join.sh installs it, and run the way the unit runs it.
    asRoot(['install', '-D', '-m', '0755', BIN ?? '', bin])
    const run = ['sh', '-c', SUPERVISOR, bin, config, releaseKey]
    const out = openSync(join(dir, 'blocklyd.log'), 'a')
    supervisor = Bun.spawn(SUDO ? ['sudo', '-n', ...run] : run, { stdout: out, stderr: out })
  }, 60_000)

  afterAll(async () => {
    supervisor?.kill('SIGKILL')
    const pkill = ['pkill', '-KILL', '-f', '--', `--config ${config}`]
    spawnSync(SUDO ? 'sudo' : 'pkill', SUDO ? ['-n', ...pkill] : pkill.slice(1))
    endpoint?.kill('SIGTERM')
    await endpoint?.exited
    await drop?.()
    if (env.BLOCKLYD_E2E_KEEP === '1') console.warn(`kept ${dir}`)
    else asRoot(['rm', '-rf', dir])
  })

  test('a node on an older blocklyd is offered the endpoint’s, upgrades, and beats healthy on it', async () => {
    const offered = await firstEvent(db, 'node.upgrade_offered')
    expect(offered.data).toMatchObject({ from: '0.0.1', version })
    expect((await firstEvent(db, 'node.upgraded')).data.version).toBe(version)
    await healthyOn(version)
    expect(sha(bin)).toBe(sha(BIN ?? ''))
    expect(readFileSync(join(dir, 'blocklyd.log'), 'utf8')).toContain('the upgrade came up')
  }, 120_000)

  test('a blocklyd that says it is newer and can’t start is put back, and the node says why', async () => {
    const [before] = await nodeSummaries(db, THRESHOLDS)
    const broken = join(dir, 'broken-blocklyd')
    writeFileSync(broken, '#!/bin/sh\n[ "$1" = --version ] && { echo "blocklyd 99.0.0"; exit 0; }\nexit 1\n')
    chmodSync(broken, 0o755)
    // The endpoint reads its release once, as it starts.
    endpoint?.kill('SIGTERM')
    await endpoint?.exited
    endpoint = await startServingEndpoint(args, url, signed(broken))
    const failed = await firstEvent(db, 'node.upgrade_failed')
    expect(failed.data).toMatchObject({ version: '99.0.0' })
    // It fails its ExecStartPre (doctor --preflight), so it never runs.
    expect(failed.data.reason).toContain("blocklyd 99.0.0 didn't start (exit-code) before it came up")
    await healthyOn(version)
    expect(sha(bin)).toBe(sha(BIN ?? ''))
    expect(readFileSync(join(dir, 'blocklyd.log'), 'utf8')).toContain(`put blocklyd ${version} back`)
    const [after] = await nodeSummaries(db, THRESHOLDS)
    expect(after?.id).toBe(before?.id ?? '')
    expect(after?.upgrade_state).toBe('failed')
  }, 120_000)
})
