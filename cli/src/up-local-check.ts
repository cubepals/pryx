// SPDX-License-Identifier: FSL-1.1-ALv2
/** Verify the selected local backend before installing configuration or starting services.
 * Bootstrap probes are read-only; distribution verification waits for the signed full bundle. */
import { existsSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { hostProbe, probeBinary } from '../../deploy/host-probe.ts'
import { imageTier } from '../../deploy/image-tier.ts'
import type { Preset } from './up-preset.ts'
import { localProblem } from './up-preset.ts'

export async function inspectLocal(preset: Preset, bundle = process.env.PRYX_BUNDLE_DIR, temporary?: string) {
  const binary = bundle ? join(bundle, 'bin/pryxd') : probeBinary()
  if (!binary) throw new Error('The bundled pryxd is missing; reinstall with the one-line pryx installer')
  const env = {
    ...process.env,
    ...(bundle ? { PATH: `${join(bundle, 'bin')}:${process.env.PATH ?? ''}` } : {}),
  }
  const probe = await hostProbe(binary, env)
  const agent = bundle ? join(bundle, 'bin/pryx-agent') : join(dirname(binary), 'pryx-agent')
  const result = Bun.spawnSync([agent, 'process-probe'], { stdin: 'ignore', stdout: 'pipe', stderr: 'pipe' })
  if (result.exitCode)
    throw new Error(
      'The release agent could not probe filesystem confinement; check its executable and host permissions',
    )
  const filesystem = (JSON.parse(result.stdout.toString()) as { filesystem: boolean }).filesystem === true
  const engine = probe.capabilities.docker?.state === 'on'
  const firewall = probe.capabilities['host-firewall']?.state
  const canContainer = preset === 'container' && engine && (firewall === 'on' || firewall === 'off')
  if (process.env.PRYX_DEFAULT_TIER === 'container' && !canContainer)
    throw new Error(
      'Container tier needs the engine and host firewall permissions; run the server preset on the host or select the process tier',
    )
  const tier = imageTier(canContainer, process.env.PRYX_DEFAULT_TIER)
  const problem = localProblem(preset, probe, filesystem, tier)
  if (problem) throw new Error(problem)
  if (tier === 'wasm') {
    if (!/glibc|gnu libc|gnu c library/i.test(probe.cLibrary))
      throw new Error('This wasm distribution needs an x86_64 glibc host; use process on a Landlock host')
    if (bundle && temporary && existsSync(join(bundle, 'agentos/distribution.json'))) {
      const config = join(temporary, 'wasm-preflight.toml')
      writeFileSync(
        config,
        `deployment_id = "pryx"\nruntime = "wasm"\nstate_dir = ${JSON.stringify(join(temporary, 'node'))}\n[wasm]\ndistribution = ${JSON.stringify(join(bundle, 'agentos'))}\n`,
        { mode: 0o600, flag: 'wx' },
      )
      const doctor = Bun.spawnSync([binary, 'doctor', '--config', config], {
        env,
        stdin: 'ignore',
        stdout: 'pipe',
        stderr: 'pipe',
      })
      if (doctor.exitCode)
        throw new Error('The wasm distribution failed its runtime doctor; use process on a Landlock host')
    }
  }
  return { probe, tier }
}
