// SPDX-License-Identifier: FSL-1.1-ALv2
/** Select an unattended installation preset. Local bundles and system-wide provisioning
 * have separate lifecycles; check mode never downloads or writes configuration. */
import { existsSync } from 'node:fs'
import { RELEASE_MACHINES } from '../../control/src/bundle-release.ts'
import { probeBinary } from '../../deploy/host-probe.ts'
import { experimentalUp, requestsLab } from './experimental-release.ts'
import { joinHost } from './join.ts'
import { option } from './sandbox.ts'
import { dnsDomain } from './up-acme-config.ts'
import { checkHostRuntime } from './up-host.ts'
import { localPaths, runInstalled, upLocal } from './up-local.ts'
import { inspectLocal } from './up-local-check.ts'
import { localRequirements } from './up-local-policy.ts'
import { readInstallation } from './up-local-record.ts'
import { selectPreset } from './up-preset.ts'
import { upPlan, up as upServer } from './up-server.ts'

export { runInstallCommand, upPlan } from './up-server.ts'

export async function up(args: string[]) {
  if (
    args.includes('--installer-arguments') ||
    (!args.includes('--run-installed') && (requestsLab(args) || args.includes('--feature')))
  )
    return await experimentalUp(args, upPreset)
  return await upPreset(args)
}

async function upPreset(args: string[], onLocalReady?: () => void) {
  const preset = selectPreset(option(args, '--preset'))
  if (args.includes('--preflight')) {
    localRequirements()
    if (preset === 'server') {
      const plan = upPlan(dnsDomain(option(args, '--domain') ?? process.env.PRYX_DOMAIN ?? 'localhost'))
      if (!plan.linux || !plan.root || !plan.systemd || !(plan.architecture in RELEASE_MACHINES))
        throw new Error('Server preset requires Linux on x86_64 or aarch64, root and systemd')
      checkHostRuntime()
    } else if (preset !== 'node')
      await inspectLocal(preset, { installation: readInstallation(localPaths(preset)) })
    return
  }
  if (args.includes('--check')) {
    if (preset === 'server' || args.includes('--image')) return { preset, ...(await upServer(args)) }
    const paths = localPaths(preset)
    let checked: unknown
    try {
      checked = probeBinary()
        ? await inspectLocal(preset, { installation: readInstallation(paths) })
        : { requirement: 'Install the signed release bootstrap to measure backend availability' }
    } catch (error) {
      checked = { requirement: error instanceof Error ? error.message : 'Host probe failed' }
    }
    return {
      ...upPlan(option(args, '--domain') ?? 'localhost'),
      preset,
      paths,
      require: localRequirements(readInstallation(paths)?.require),
      probe: checked,
      changes: false,
    }
  }
  if (preset === 'server' || args.includes('--image')) return await upServer(args)
  if (preset === 'node') {
    const token = option(args, '--join-token') ?? process.env.PRYX_JOIN_TOKEN
    if (!token) throw new Error('The node preset needs a fleet join token; set PRYX_JOIN_TOKEN')
    return await joinHost(token, { args: [] })
  }
  if (args.includes('--run-installed')) return await runInstalled(preset)
  if (!existsSync(process.env.PRYX_BUNDLE_DIR ?? '') && !probeBinary())
    throw new Error('Install the release bootstrap with the one-line installer before pryx up')
  return await upLocal(args, preset, false, onLocalReady)
}
