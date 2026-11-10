// SPDX-License-Identifier: FSL-1.1-ALv2
/** The daemon's probe document for installation and startup summaries. This does not infer
 * sandbox guarantees: only the serving node and placement can promise those. */
import { existsSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { SECOND_MS } from '../control/src/durations.ts'

export type HostProbe = {
  architecture: string
  cLibrary: string
  landlockAbi: number | null
  capabilities: Record<string, { state: 'on' | 'off' | 'unknown'; evidence: string; action: string }>
}
export function probeBinary() {
  return (
    process.env.PRYX_PRYXD_BIN ??
    [
      process.env.PRYX_BUNDLE_DIR && join(process.env.PRYX_BUNDLE_DIR, 'bin/pryxd'),
      join(dirname(process.execPath), 'pryxd'),
      Bun.which('pryxd'),
      '/opt/pryx/pryxd',
    ].find((path): path is string => Boolean(path && existsSync(path)))
  )
}
export async function hostProbe(binary: string): Promise<HostProbe> {
  const child = Bun.spawn([binary, 'probe', '--json'], { stdin: 'ignore', stdout: 'pipe', stderr: 'pipe' })
  const timer = setTimeout(() => child.kill('SIGKILL'), 30 * SECOND_MS)
  try {
    const [stdout, , exit] = await Promise.all([
      new Response(child.stdout).text(),
      new Response(child.stderr).text(),
      child.exited,
    ])
    if (exit) throw new Error('Host probe failed; check executable and host permissions')
    return JSON.parse(stdout) as HostProbe
  } finally {
    clearTimeout(timer)
  }
}
export function capabilitySummary(probe: HostProbe) {
  return Object.fromEntries(
    Object.entries(probe.capabilities).map(([name, capability]) => [
      name,
      capability.state === 'on' ? { state: 'on' } : capability,
    ]),
  )
}
