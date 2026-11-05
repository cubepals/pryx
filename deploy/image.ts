// SPDX-License-Identifier: FSL-1.1-ALv2
/** One-image supervisor. Control remains the lifecycle owner, edge owns ingress and pryxd owns
 * workloads. Embedded services listen on loopback; the gateway is the only public listener. */
import { closeSync, existsSync, ftruncateSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { availableParallelism, totalmem } from 'node:os'
import { join } from 'node:path'
import { loadConfig } from '../control/src/config.ts'
import { SECOND_MS } from '../control/src/durations.ts'
import { HTTP_FRONT_PORT } from '../control/src/edge-headers.ts'
import { startControl } from '../control/src/main.ts'
import { createToken, nodeSummaries } from '../control/src/registry.ts'
import { startEdge } from '../edge/src/agent.ts'
import { edgeConfig } from '../edge/src/config.ts'
import { front } from '../edge/src/front.ts'
import {
  createPrivateFile,
  credentials,
  embedded,
  IMAGE_NODE_CONFIG,
  privateLog,
  until,
} from './image-state.ts'

let startupStage = 'configuration',
  diagnosticRoot: string | undefined
async function serve() {
  const base = process.env.PRYX_STATE_DIR ?? '/var/lib/pryx'
  const root = process.getuid?.() === 0 ? base : join(base, `uid-${process.getuid?.()}`)
  const publicPort = Number(process.env.PORT ?? 8080)
  const origin =
    process.env.PRYX_PUBLIC_URL ?? process.env.RENDER_EXTERNAL_URL ?? `http://127.0.0.1:${publicPort}`
  const parsedOrigin = new URL(origin)
  if (
    !['http:', 'https:'].includes(parsedOrigin.protocol) ||
    parsedOrigin.pathname !== '/' ||
    parsedOrigin.search ||
    parsedOrigin.hash ||
    parsedOrigin.username ||
    parsedOrigin.password
  )
    throw new Error('PRYX_PUBLIC_URL must be an HTTP origin')
  startupStage = 'credentials'
  const values = await credentials(root),
    children: Bun.Subprocess[] = []
  diagnosticRoot = root
  startupStage = 'runtime'
  const docker =
    existsSync('/var/run/docker.sock') &&
    Bun.spawnSync(['docker', 'version', '--format', '{{.Server.Version}}'], {
      stdout: 'ignore',
      stderr: 'ignore',
    }).exitCode === 0
  const tier = docker ? 'container' : 'process'
  let memory = totalmem()
  try {
    const limit = Number(readFileSync('/sys/fs/cgroup/memory.max', 'utf8').trim())
    if (Number.isFinite(limit)) memory = Math.min(memory, limit)
  } catch {}
  const reservedMemory = Number(
    process.env.PRYX_RESERVED_MEMORY_MB ??
      Math.max(256, Math.min(2048, Math.floor(memory / (1024 * 1024 * 4)))),
  )
  const reservedCpu = Number(
    process.env.PRYX_RESERVED_CPU_MILLIS ?? Math.min(1000, Math.floor((availableParallelism() * 1000) / 4)),
  )
  const stopChildren = async () => {
    for (const child of children.toReversed()) child.kill('SIGTERM')
    await Promise.race([Promise.all(children.map((child) => child.exited)), Bun.sleep(10 * SECOND_MS)])
    for (const child of children) if (child.exitCode === null) child.kill('SIGKILL')
  }
  let stop = stopChildren
  let stopping = false
  const shutdown = async (code: number) => {
    if (stopping) return
    stopping = true
    await stop()
    process.exit(code)
  }
  process.once('SIGTERM', () => {
    void shutdown(0)
  })
  process.once('SIGINT', () => {
    void shutdown(0)
  })
  startupStage = 'storage'
  const storage = await embedded(root, values, children)
  startupStage = 'control'
  const config = loadConfig({
    ...process.env,
    DATABASE_URL: storage.database,
    PRYX_CA_CERT: values.caCert,
    PRYX_CA_KEY: values.caKey,
    PRYX_OPERATOR_TOKEN: values.operator,
    PRYX_EDGE_TOKEN: values.edge,
    PRYX_BOOTSTRAP_API_KEY: values.key,
    PRYX_DEFAULT_ISOLATION: tier,
    PRYX_PUBLIC_URL: origin,
    PRYX_DEPLOYMENT: 'pryx',
    PRYX_NODE_LISTEN: '127.0.0.1:8443',
    PRYX_OPERATOR_LISTEN: '127.0.0.1:8081',
    PRYX_API_LISTEN: '127.0.0.1:8082',
    PRYX_HEARTBEAT_SECONDS: '1',
    PRYX_SUSPEND: 'on',
    S3_ENDPOINT: storage.s3.endpoint,
    S3_BUCKET: storage.s3.bucket,
    S3_ACCESS_KEY: storage.s3.accessKeyId,
    S3_SECRET_KEY: storage.s3.secretAccessKey,
  })
  const control = await startControl(config, {
    gatewayHealth: () =>
      Response.json({ status: 'ok', isolation: tier, sharedUid: process.getuid?.() !== 0 }),
  })
  const nodeRoot = join(root, 'node'),
    identity = join(root, 'private', 'fleet')
  mkdirSync(nodeRoot, { recursive: true, mode: 0o711 })
  mkdirSync(identity, { recursive: true, mode: 0o700 })
  writeFileSync(join(identity, 'ca.pem'), values.caCert, { mode: 0o600 })
  const tokenPath = join(identity, 'token')
  if (!existsSync(join(nodeRoot, 'identity', 'node.json'))) {
    const token = await createToken(control.db, { regionKey: 'default', ttlSeconds: 600, createdBy: 'image' })
    writeFileSync(tokenPath, token.token, { mode: 0o600 })
  }
  const nodeConfig = join(root, 'private', IMAGE_NODE_CONFIG)
  const user =
    process.getuid?.() === 0
      ? '1000:1000'
      : `${process.getuid?.()}:${process.getgid?.() || process.getuid?.()}`
  writeFileSync(
    nodeConfig,
    `deployment_id = "pryx"
runtime = "${tier}"
state_dir = ${JSON.stringify(nodeRoot)}
[api]
listen = "127.0.0.1:7443"
[ops]
listen = "127.0.0.1:7070"
[fleet]
url = "https://127.0.0.1:8443"
ca = ${JSON.stringify(join(identity, 'ca.pem'))}
enrollment_token_file = ${JSON.stringify(tokenPath)}
heartbeat_seconds = 1
${tier === 'process' ? `[capacity]\nreserved_memory_mb = ${reservedMemory}\nreserved_cpu_millis = ${reservedCpu}` : ''}
[workloads]
agent_public_origin = ${JSON.stringify(origin)}
user = "${user}"
allowed_images = ["alpine:", "ghcr.io/", "docker.io/", "mirror.gcr.io/"]
[network]
edge_ips = ["127.0.0.1"]
control_ips = ["127.0.0.1"]
`,
    { mode: 0o600 },
  )
  startupStage = 'enrollment'
  const log = privateLog(root, 'node')
  children.push(Bun.spawn(['/pryxd', 'serve', '--config', nodeConfig], { stdout: log, stderr: log }))
  await until('Local node', async () => {
    const nodes = await nodeSummaries(control.db, { suspectSeconds: 15, unavailableSeconds: 45 })
    return nodes.some((node) => node.derivedHealth === 'healthy')
  })
  if (docker) {
    const isolate = Bun.spawn(['/pryxd', 'isolate', '--config', nodeConfig], {
      stdout: 'ignore',
      stderr: privateLog(root, 'isolate'),
    })
    if (await isolate.exited) throw new Error('Container tier host firewall policy could not be installed')
  }
  await control.runtime.refreshRegistry()
  const edge = await startEdge({
    ...edgeConfig({
      PRYX_CONTROL_URL: control.apiUrl,
      PRYX_EDGE_TOKEN: values.edge,
      PRYX_EDGE_HOST: '127.0.0.1',
      PRYX_EDGE_HTTP_ONLY: '1',
    }),
    httpPort: 0,
    metricsPort: 0,
  })
  if (control.edge.agents) control.edge.agents.localOrigin = origin
  edge.agents.useControl(control.apiUrl)
  startupStage = 'gateway'
  const gateway = await front({
    host: '0.0.0.0',
    port: publicPort,
    kind: 'http',
    routePort: HTTP_FRONT_PORT,
    agents: edge.agents,
    table: edge.table,
    gate: edge.gate,
    hold: edge.hold,
    holdMs: edgeConfig({ PRYX_EDGE_TOKEN: values.edge }).holdMs,
    greetingMs: SECOND_MS * 5,
    bufferBytes: 64 * 1024,
    metrics: edge.metrics,
    session: () => {},
    control: { destination: edge.agents.destination() as string, paths: /^\// },
  })
  stop = async () => {
    gateway.closeClients()
    await new Promise<void>((resolve) => gateway.close(() => resolve()))
    await edge.stop()
    await control.stop()
    await stopChildren()
  }
  for (const child of children)
    void child.exited.then(() => {
      if (!stopping) void shutdown(1)
    })
  console.log(
    `pryx listening on ${publicPort}; isolation=${tier}; ${process.getuid?.() === 0 ? 'per-sandbox UIDs' : 'shared UID'}`,
  )
  const printed = join(root, 'private', 'key-printed')
  if (!existsSync(printed)) {
    createPrivateFile(printed, 'printed\n')
    console.log(JSON.stringify({ apiUrl: origin, apiKey: values.key }))
  }
}
if (process.argv.length > 2) {
  const child = Bun.spawn(['/pryxd', ...process.argv.slice(2)], {
    stdin: 'inherit',
    stdout: 'inherit',
    stderr: 'inherit',
  })
  process.exit(await child.exited)
}
await serve().catch((error: unknown) => {
  if (diagnosticRoot) {
    try {
      const fd = privateLog(diagnosticRoot, 'startup')
      try {
        ftruncateSync(fd, 0)
        writeFileSync(
          fd,
          String(error instanceof Error ? error.stack : 'Unknown startup failure').slice(0, 16 * 1024),
        )
      } finally {
        closeSync(fd)
      }
    } catch {}
  }
  const code = error && typeof error === 'object' && 'code' in error ? String(error.code) : ''
  console.error(
    JSON.stringify({
      error: 'startup_failed',
      stage: startupStage,
      ...(/^[A-Z][A-Z0-9_]{0,47}$/.test(code) ? { code } : {}),
    }),
  )
  process.exit(1)
})
