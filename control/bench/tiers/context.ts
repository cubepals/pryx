// SPDX-License-Identifier: FSL-1.1-ALv2
/** Real deployment contexts and process-tree memory for the existing CI comparison jobs. */
import { Pryx } from '../../../sdk/typescript/src/index.ts'
import { agentDeployment } from '../../e2e/agent-context.ts'
import { TIER_BENCH_CPU_OVERCOMMIT, TIER_BENCH_UNIT } from '../../e2e/runtime-config.ts'
import { until } from '../../e2e/waits.ts'
import { SECOND_MS } from '../../src/durations.ts'
import type { Isolation } from '../../src/isolation.ts'
import { processDiagnostics } from './process-diagnostics.ts'
import { command } from './report.ts'

export type Context = Awaited<ReturnType<typeof context>>
// Explicit bounded operator profile for fifty simultaneous public HTTP requests.
export const IMAGE_BENCH_GATEWAY = {
  PRYX_EDGE_ADDRESS_GREETING: '64',
  PRYX_EDGE_ADDRESS_CONNECTIONS_BURST: '128',
  PRYX_EDGE_ADDRESS_CONNECTIONS_PER_SECOND: '64',
}
export async function context(mode: string, tier: Isolation, override = true) {
  if (mode === 'one-box' || tier === 'container') {
    const c = await agentDeployment(tier)
    return {
      sdk: c.sdk,
      env: c.d.env,
      topology: 'host node, control and edge; container requires the host engine',
      memory: () =>
        memory(
          c.d.children.map(({ child }) => child.pid),
          c.d.control.config.deployment,
        ),
      close: c.close,
      diagnostics: () => processDiagnostics(c.d),
    }
  }
  const name = `pryx-k14-${tier}${override ? '' : '-default'}`
  await command([
    'docker',
    'run',
    '-d',
    '--name',
    name,
    '-p',
    '8080:8080',
    ...(override ? ['-e', `PRYX_DEFAULT_TIER=${tier}`] : []),
    ...(mode === 'image'
      ? [
          '-e',
          `PRYX_UNIT_MEMORY_MB=${TIER_BENCH_UNIT.memoryMb}`,
          '-e',
          `PRYX_UNIT_CPU_MILLIS=${TIER_BENCH_UNIT.cpuMillis}`,
          '-e',
          `PRYX_CPU_OVERCOMMIT=${TIER_BENCH_CPU_OVERCOMMIT}`,
          ...Object.entries(IMAGE_BENCH_GATEWAY).flatMap(([key, value]) => ['-e', `${key}=${value}`]),
        ]
      : []),
    process.env.PRYX_IMAGE_TEST ?? 'pryx-static-test',
  ])
  try {
    await until(
      'ordinary tier image',
      async () => {
        const response = await fetch('http://127.0.0.1:8080/healthz').catch(() => undefined)
        if (!response?.ok) return null
        const health = (await response.json()) as { isolation: string }
        if (health.isolation !== tier) throw new Error('Image isolation differs')
        return health
      },
      60 * SECOND_MS,
    )
    const config = JSON.parse(await command(['docker', 'inspect', '--format', '{{json .HostConfig}}', name]))
    if (config.Privileged || config.CapAdd?.length || config.SecurityOpt?.length || config.Binds?.length)
      throw new Error('Ordinary image had an isolation override or host mount')
    const logs = await command(['docker', 'logs', name])
    const keys = logs.split('\n').flatMap((line) => {
      try {
        const value = JSON.parse(line)
        return typeof value.apiKey === 'string' ? [value.apiKey] : []
      } catch {
        return []
      }
    })
    if (keys.length !== 1) throw new Error('Image did not print exactly one initial key')
    const env = { ...process.env, PRYX_API_URL: 'http://127.0.0.1:8080', PRYX_API_KEY: keys[0] }
    const pid = Number((await command(['docker', 'inspect', '--format', '{{.State.Pid}}', name])).trim())
    return {
      sdk: new Pryx({ url: env.PRYX_API_URL, key: keys[0] as string }),
      env,
      topology: 'ordinary docker run; no capabilities, security options, socket or host mounts',
      diagnostics: undefined,
      memory: () => memory([pid]),
      close: async () => {
        await command(['docker', 'rm', '-fv', name])
      },
    }
  } catch (error) {
    console.error(
      (await command(['docker', 'logs', name]))
        .replace(/("apiKey"\s*:\s*)"[^"]*"/g, '$1"[redacted]"')
        .slice(-8192),
    )
    await command(['docker', 'rm', '-fv', name])
    throw error
  }
}
async function memory(roots: number[], deployment?: string) {
  if (deployment) {
    const ids = (await command(['docker', 'ps', '-q', '--filter', `label=blocklyd.deployment=${deployment}`]))
      .trim()
      .split(/\s+/)
      .filter(Boolean)
    if (ids.length) {
      const pids = await command(['docker', 'inspect', '--format', '{{.State.Pid}}', ...ids])
      roots.push(...pids.trim().split(/\s+/).map(Number))
    }
  }
  const value = JSON.parse(
    await command([
      ...(process.getuid?.() === 0 ? [] : ['sudo', '-n']),
      'python3',
      'control/tools/tier-memory.py',
      ...roots.map(String),
    ]),
  ) as { pssBytes: number; missing: number[] }
  if (roots.some((pid) => value.missing.includes(pid))) throw new Error('Memory root disappeared')
  return value.pssBytes
}
