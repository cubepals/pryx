// SPDX-License-Identifier: FSL-1.1-ALv2
/** Container/process cohorts on one CI host. Every operation validates its result; failed
 * cohorts retain partial samples and fail the step. Credentials never enter the report. */
import { writeFileSync } from 'node:fs'
import type { AddressInfo } from 'node:net'
import { cpus, release, totalmem } from 'node:os'
import { StatusCodes } from 'http-status-codes'
import { startEdge } from '../../edge/src/agent.ts'
import { edgeConfig } from '../../edge/src/config.ts'
import { Pryx, type Sandbox } from '../../sdk/typescript/src/index.ts'
import { AppRequests } from '../e2e/app-requests.ts'
import { deployment, ready } from '../e2e/deployment.ts'
import { SECOND_MS } from '../src/durations.ts'

const phase = process.argv[2]
if (phase !== 'before' && phase !== 'after') throw new Error('process-speed.ts before|after output.json')
if (!ready) throw new Error('Process cohorts require the CI Docker/Postgres/S3 prerequisites')
const N = 30,
  DATA_BYTES = 100_000_000,
  image = process.env.PRYX_IMAGE_TEST ?? 'pryx-static-test'
const samples: Record<string, number[]> = {},
  absent: Record<string, string> = {}
const report: Record<string, unknown> = {
  phase,
  comparison: 'container and process tiers on the same source and CI host',
  source: Bun.spawnSync(['git', 'rev-parse', 'HEAD']).stdout.toString().trim(),
  host: {
    cpu: cpus()[0]?.model,
    cores: cpus().length,
    kernel: release(),
    memoryBytes: totalmem(),
    bun: Bun.version,
  },
  dataset: { bytes: DATA_BYTES, byte: 42 },
  samples,
  absent,
}
async function command(args: string[], env = process.env) {
  const child = Bun.spawn(args, { env, stdout: 'pipe', stderr: 'pipe' })
  const [stdout] = await Promise.all([new Response(child.stdout).text(), new Response(child.stderr).text()])
  if (await child.exited) throw new Error(`${args[0]} failed`)
  return stdout
}
async function measure(name: string, operation: (i: number) => Promise<void | (() => Promise<void>)>) {
  const raw = samples[name] ?? []
  samples[name] = raw
  for (let i = 0; i < N; i++) {
    const start = performance.now()
    const cleanup = await operation(i)
    raw.push(performance.now() - start)
    if (cleanup) await cleanup()
  }
}
async function boot(name: string, user?: string) {
  const start = performance.now()
  await command([
    'docker',
    'run',
    '-d',
    '--name',
    name,
    '-p',
    '8080:8080',
    ...(user ? ['--user', user] : []),
    image,
  ])
  const facts = JSON.parse(await command(['docker', 'exec', name, 'python3', '/opt/pryx/process-facts.py']))
  const policy = JSON.parse(
    await command(['docker', 'exec', name, '/usr/local/lib/pryx/pryx-agent', 'process-probe']),
  )
  const hostConfig = JSON.parse(
    await command(['docker', 'inspect', '--format', '{{json .HostConfig}}', name]),
  )
  report.capabilities = facts
  report.policy = policy
  if (
    hostConfig.Privileged ||
    hostConfig.CapAdd?.length ||
    hostConfig.SecurityOpt?.length ||
    hostConfig.Binds?.length
  )
    throw new Error('Process proof used an isolation override')
  if (
    facts.engineSocket ||
    facts.writableCgroup ||
    Object.values(facts.canCreateNamespaces).some(Boolean) ||
    facts.seccomp !== 2
  )
    throw new Error('Ordinary-container capability facts did not match the brief')
  if (!policy.filesystem) throw new Error('Required Landlock policy is unavailable in the ordinary image')
  report.capabilities = facts
  report.policy = policy

  const deadline = performance.now() + 30 * SECOND_MS
  while (performance.now() < deadline) {
    const response = await fetch('http://127.0.0.1:8080/healthz').catch(() => undefined)
    if (response?.ok) {
      const state = (await response.json()) as { isolation: string; sharedUid: boolean }
      if (state.isolation !== 'process' || state.sharedUid !== Boolean(user))
        throw new Error('Wrong isolation announcement')
      const elapsed = performance.now() - start
      const logs = await command(['docker', 'logs', name])
      const keys = logs.split('\n').flatMap((line) => {
        try {
          const value = JSON.parse(line)
          return typeof value.apiKey === 'string' ? [value.apiKey] : []
        } catch {
          return []
        }
      })
      if (keys.length !== 1) throw new Error('Initial key was not printed exactly once')
      return { key: keys[0] as string, elapsed, facts, policy }
    }
    await Bun.sleep(SECOND_MS / 100)
  }
  throw new Error('Ordinary image did not answer its API')
}
async function remove(name: string) {
  await command(['docker', 'rm', '-fv', name])
}

async function operations(sdk: Pryx, origin: string, env: Record<string, string | undefined>) {
  const apps = new AppRequests(origin)
  const box = await sdk.sandbox({
    ...(phase === 'before' ? { image: 'alpine:pryx-tls' } : {}),
    resources: { memoryMb: 512 },
    sparePorts: 1,
  })
  try {
    const view = await sdk.api.get(box.id)
    if (phase === 'after' && view.isolation !== 'process')
      throw new Error('Sandbox did not report process isolation')
    await box.run('true')
    await measure('run_edge', async () => {
      if ((await box.run('true')).code !== 0) throw new Error('Run failed')
    })
    samples.first_byte = []
    for (let i = 0; i < N; i++) {
      const start = performance.now(),
        run = box.run('echo hi'),
        stream = run.stream()
      if ((await stream.next()).value?.data !== 'hi\n') throw new Error('First byte differs')
      samples.first_byte.push(performance.now() - start)
      while (!(await stream.next()).done) {}
      if ((await run).code !== 0) throw new Error('Echo did not exit')
    }
    await box.write('index.html', 'ready')
    await measure('start_expose_200', async () => {
      const web = await box.start(
        phase === 'before'
          ? ['httpd', '-f', '-p', '3000', '-h', '/data']
          : ['python3', '-m', 'http.server', '3000', '--bind', '127.0.0.1', '--directory', '.'],
      )
      try {
        const url = await box.expose(3000),
          end = performance.now() + 5 * SECOND_MS
        let success = false
        while (performance.now() < end) {
          try {
            const response = phase === 'before' ? await apps.get(url) : await fetch(url)
            if (response.status === StatusCodes.OK && (await response.text()) === 'ready') {
              success = true
              break
            }
          } catch {}
          await Bun.sleep(SECOND_MS / 1000)
        }
        if (!success) throw new Error('Exposed HTTP content differs')
      } catch (error) {
        const current = await box.agent.process(web.id)
        if (current.view.running) await web.kill()
        const result = await web.wait()
        let output = ''
        for await (const chunk of web.stream()) {
          output += chunk.data.slice(0, Math.max(0, 4096 - output.length))
          if (output.length >= 4096) break
        }
        report.exposeFailure = { result, output }
        throw error
      }
      return async () => {
        await box.unexpose(3000)
        await web.kill()
        await web.wait()
      }
    })
    const chunk = new Uint8Array(1024 * 1024).fill(42)
    await box.write(
      'payload',
      (async function* () {
        let left = DATA_BYTES
        while (left) {
          const part = chunk.subarray(0, Math.min(left, chunk.length))
          yield part
          left -= part.length
        }
      })(),
    )
    if ((await box.files.stat('payload')).size !== DATA_BYTES) throw new Error('Dataset differs')
    await measure('snapshot_100mb', async () => {
      const snapshot = await box.snapshot()
      return async () => {
        await snapshot.delete()
      }
    })
    const snapshot = await box.snapshot()
    try {
      await measure('fork_100mb', async () => {
        const fork = await snapshot.fork()
        return async () => {
          try {
            if ((await fork.files.stat('payload')).size !== DATA_BYTES)
              throw new Error('Fork dataset differs')
          } finally {
            await fork.delete()
          }
        }
      })
    } finally {
      await snapshot.delete()
    }
    await measure('sleep_signal', async () => {
      await box.sleep()
      return async () => {
        await box.wake()
      }
    })
    // Actual signal wake includes the lifecycle request; each sleep is outside the timer.
    samples.wake_signal = []
    for (let i = 0; i < N; i++) {
      await box.sleep()
      const start = performance.now()
      await box.wake()
      samples.wake_signal.push(performance.now() - start)
    }
    await measure('pryx_run_echo', async () => {
      const output = await command(
        [
          process.execPath,
          'cli/src/main.ts',
          'run',
          ...(phase === 'before' ? ['--image', 'alpine:pryx-tls'] : []),
          '--',
          'echo',
          'hello',
        ],
        env,
      )
      if (output.trim() !== 'hello') throw new Error('CLI output differs')
    })
    if (phase === 'after') await acceptance(sdk, box, env)
  } finally {
    apps.close()
    await box.delete()
  }
}
async function acceptance(sdk: Pryx, box: Sandbox, env: Record<string, string | undefined>) {
  const refused = Bun.spawn(
    [process.execPath, 'cli/src/main.ts', 'run', '--image', 'alpine:3.22', '--', 'echo', 'hello'],
    { env, stdout: 'pipe', stderr: 'pipe' },
  )
  const [out, err] = await Promise.all([
    new Response(refused.stdout).text(),
    new Response(refused.stderr).text(),
  ])
  if (
    (await refused.exited) === 0 ||
    out.includes('hello') ||
    !err.includes('this host runs the process tier; images are not available')
  )
    throw new Error(
      `CLI image refusal differs: ${JSON.stringify({ code: refused.exitCode, stdout: out.slice(0, 256), stderr: err.slice(0, 2048) })}`,
    )
  await box.stop()
  await box.write('stopped', 'before')
  await box.start()
  await box.write('stopped', 'after')
  if ((await box.read('stopped')) !== 'after') throw new Error('Stopped ownership failed')
  if ((await box.run(['node', '-e', 'console.log("node-ready")'])).stdout.trim() !== 'node-ready')
    throw new Error('Bundled Node failed')
  const other = await sdk.sandbox()
  try {
    await other.write('secret', 'private')
    const path = (await other.run('pwd')).stdout.trim()
    if ((await box.run(['cat', `${path}/secret`])).code === 0) throw new Error('Another sandbox was readable')
  } finally {
    await other.delete()
  }
  await box.network('none')
  if (
    (
      await box.run([
        'python3',
        '-c',
        "import socket\ntry:socket.create_connection(('127.0.0.1',8080))\nexcept PermissionError:print('filtered')\nelse:raise Exception('escaped')",
      ])
    ).stdout.trim() !== 'filtered'
  )
    throw new Error('Network-off failed')
  await box.network('internet')
  try {
    await sdk.sandbox({ image: 'alpine:3.22' })
    throw new Error('Process host accepted an image')
  } catch (error) {
    if (
      !(
        error instanceof Error &&
        'status' in error &&
        error.status === StatusCodes.UNPROCESSABLE_ENTITY &&
        'details' in error &&
        JSON.stringify(error.details).includes('this host runs the process tier; images are not available')
      )
    )
      throw error
  }
}

let close = async () => {},
  container: string | undefined
try {
  if (phase === 'before') {
    absent.image_start = 'The preceding image contains only the daemon and has no one-image API; n=0'
    const d = await deployment({ agent: true }),
      edge = await startEdge({
        ...edgeConfig({
          PRYX_CONTROL_URL: d.control.apiUrl,
          PRYX_EDGE_TOKEN: d.control.config.edgeToken,
          PRYX_EDGE_HOST: '127.0.0.1',
        }),
        httpPort: 0,
        tlsPort: 0,
        metricsPort: 0,
      })
    close = async () => {
      await edge.stop()
      await d.close()
    }
    const origin = `http://127.0.0.1:${(edge.listeners.get('http:80')?.address() as AddressInfo).port}`
    if (d.control.edge.agents) d.control.edge.agents.localOrigin = origin
    await operations(new Pryx({ url: d.control.apiUrl, key: d.credential.key }), origin, d.env)
  } else {
    report.imageId = (await command(['docker', 'image', 'inspect', '--format', '{{.Id}}', image])).trim()
    samples.image_start = []
    for (let i = 0; i < N; i++) {
      container = `pryx-process-${i}`
      const started = await boot(container)
      samples.image_start.push(started.elapsed)
      report.capabilities = started.facts
      report.policy = started.policy
      if (i !== N - 1) {
        await remove(container)
        container = undefined
        continue
      }
      report.agentSha256 = (
        await command(['docker', 'exec', container, 'sha256sum', '/usr/local/lib/pryx/pryx-agent'])
      ).split(' ')[0]
      const env = { ...process.env, PRYX_API_URL: 'http://127.0.0.1:8080', PRYX_API_KEY: started.key }
      await operations(new Pryx({ url: env.PRYX_API_URL, key: started.key }), env.PRYX_API_URL, env)
      await command(['docker', 'restart', container])
      const end = Date.now() + 30 * SECOND_MS
      while (Date.now() < end) {
        try {
          if ((await fetch('http://127.0.0.1:8080/healthz')).ok) break
        } catch {}
        await Bun.sleep(SECOND_MS / 100)
      }
      const logs = await command(['docker', 'logs', container])
      if (logs.split('\n').filter((line) => line.includes('"apiKey"')).length !== 1)
        throw new Error('Restart printed the key again')
    }
    if (container) {
      await remove(container)
      container = undefined
    }
    container = 'pryx-process-shared'
    const shared = await boot(container, '1000:1000')
    const sdk = new Pryx({ url: 'http://127.0.0.1:8080', key: shared.key }),
      box = await sdk.sandbox()
    try {
      if ((await box.run('echo shared')).stdout.trim() !== 'shared') throw new Error('Shared UID mode failed')
    } finally {
      await box.delete()
    }
    report.sharedUidCapabilities = shared.facts
  }
  const budgets = Object.entries({
    run_edge: 20,
    first_byte: 10,
    start_expose_200: 200,
    ...(phase === 'after' ? { image_start: 10 * SECOND_MS } : {}),
  }).map(([name, limitMs]) => {
    const values = [...(samples[name] ?? [])].sort((a, b) => a - b),
      p50Ms = values[Math.floor(values.length / 2)],
      p90Ms = values[Math.ceil(values.length * 0.9) - 1]
    return {
      name,
      limitMs,
      n: values.length,
      p50Ms,
      p90Ms,
      passed: values.length === N && p90Ms !== undefined && p90Ms < limitMs,
    }
  })
  report.budgets = budgets
  if (budgets.some((budget) => !budget.passed)) throw new Error('Speed budget missed')
  report.completed = true
} catch (error) {
  report.completed = false
  report.failure = error instanceof Error ? { name: error.name, message: error.message } : 'failure'
  throw error
} finally {
  report.measurements = Object.fromEntries(
    Object.entries(samples).map(([name, raw]) => {
      const values = [...raw].sort((a, b) => a - b)
      return [
        name,
        {
          n: raw.length,
          p50Ms: values[Math.floor(values.length / 2)],
          p90Ms: values[Math.ceil(values.length * 0.9) - 1],
          samplesMs: raw,
        },
      ]
    }),
  )
  delete report.samples
  writeFileSync(process.argv[3] ?? `/tmp/process-speed-${phase}.json`, JSON.stringify(report, null, 2))
  console.log(JSON.stringify(report))
  if (container) await remove(container)
  await close()
}
