// SPDX-License-Identifier: FSL-1.1-ALv2
/** Verify the selected local backend before installing configuration or starting services.
 * Bootstrap probes are read-only; distribution verification waits for the signed full bundle. */
import { existsSync, realpathSync } from 'node:fs'
import { homedir } from 'node:os'
import { dirname, isAbsolute, join } from 'node:path'
import { containerContext } from '../../deploy/host-context.ts'
import { hostProbe, probeBinary } from '../../deploy/host-probe.ts'
import { IMAGE_NODE_CONFIG } from '../../deploy/image-state.ts'
import { imageTier } from '../../deploy/image-tier.ts'
import { configuredPort } from './up-local-network.ts'
import type { LocalInstallation } from './up-local-record.ts'
import { verifyLocalWasm } from './up-local-wasm.ts'
import type { Preset } from './up-preset.ts'
import { localProblem } from './up-preset.ts'

export async function inspectLocal(
  preset: Preset,
  options: {
    bundle?: string
    temporary?: string
    requested?: string
    installation?: LocalInstallation
  } = {},
) {
  const {
    bundle = process.env.PRYX_BUNDLE_DIR,
    temporary,
    requested = process.env.PRYX_DEFAULT_TIER,
    installation,
  } = options
  configuredPort()
  const binary = bundle ? join(bundle, 'bin/pryxd') : probeBinary()
  if (!binary) throw new Error('The bundled pryxd is missing; reinstall with the one-line pryx installer')
  const env = {
    ...process.env,
    ...(installation?.engineSocket ? { DOCKER_HOST: `unix://${installation.engineSocket}` } : {}),
    ...(preset === 'user' ? { TMPDIR: realpathSync(homedir()) } : {}),
    ...(bundle ? { PATH: `${join(bundle, 'bin')}:${process.env.PATH ?? ''}` } : {}),
  }
  const configured =
    installation?.engineSocket && !options.bundle
      ? join(installation.paths.config, IMAGE_NODE_CONFIG)
      : undefined
  const probe = await hostProbe(binary, env, configured && existsSync(configured) ? configured : undefined)
  const agent = bundle ? join(bundle, 'bin/pryx-agent') : join(dirname(binary), 'pryx-agent')
  const result = Bun.spawnSync([agent, 'process-probe'], {
    env,
    stdin: 'ignore',
    stdout: 'pipe',
    stderr: 'pipe',
  })
  if (result.exitCode)
    throw new Error(
      'The release agent could not probe filesystem confinement; check its executable and host permissions',
    )
  const filesystem = (JSON.parse(result.stdout.toString()) as { filesystem: boolean }).filesystem === true
  const selected = probe.capabilities['engine-selected']
  const engineSocket =
    installation?.engineSocket ??
    (selected?.state === 'on' && isAbsolute(selected.evidence) ? selected.evidence : undefined)
  const canContainer =
    !containerContext() &&
    probe.tiers?.container?.state === 'on' &&
    (!installation?.engineSocket || selected?.evidence === installation.engineSocket)
  if (requested === 'container' && !canContainer)
    throw new Error(
      'Container tier is unavailable in this host context; run pryx on the engine host with a supported local engine, or select an available process or wasm tier',
    )
  const gnu = /glibc|gnu libc|gnu c library/i.test(probe.cLibrary)
  const tier = imageTier(
    canContainer,
    requested ?? (!canContainer && !filesystem && gnu ? 'wasm' : undefined),
  )
  const problem = localProblem(preset, probe, filesystem, tier)
  if (problem) throw new Error(problem)
  if (tier === 'wasm') {
    if (!gnu)
      throw new Error('This wasm distribution needs an x86_64 glibc host; use process on a Landlock host')
    if (bundle && temporary && existsSync(join(bundle, 'agentos/distribution.json')))
      await verifyLocalWasm(binary, bundle, temporary, env)
  }
  return { probe, tier, engineSocket }
}
