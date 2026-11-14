// SPDX-License-Identifier: FSL-1.1-ALv2
/** Choose a host preset without changing the host. Backend selection and its guarantees stay
 * with the node; this only chooses how the same release programs are installed. */
import { existsSync } from 'node:fs'
import { containerContext } from '../../deploy/host-context.ts'
import type { HostProbe } from '../../deploy/host-probe.ts'

export const PRESETS = ['user', 'container', 'server', 'node'] as const
export type Preset = (typeof PRESETS)[number]
export function selectPreset(
  override: string | undefined,
  facts = {
    uid: process.getuid?.(),
    container: containerContext(),
    systemd: existsSync('/run/systemd/system'),
  },
): Preset {
  if (override) {
    if (!PRESETS.includes(override as Preset))
      throw new Error('--preset must be user, container, server or node')
    return override as Preset
  }
  if (facts.uid !== 0) return 'user'
  if (facts.container) return 'container'
  if (facts.systemd) return 'server'
  throw new Error(
    'Root without a container or systemd cannot start embedded PostgreSQL. Run this command as an ordinary user for the user preset',
  )
}
export function localProblem(
  preset: Preset,
  probe: HostProbe,
  filesystem: boolean,
  tier = 'process',
): string | undefined {
  if (process.platform !== 'linux') return 'Linux is required; run pryx on a Linux host'
  if (probe.architecture !== 'x86_64') return `This release needs x86_64; detected ${probe.architecture}`
  if (preset === 'user' && process.getuid?.() === 0)
    return 'The user preset needs an ordinary user; run this command without sudo'
  if (tier === 'process' && !filesystem)
    return 'Process tier requires fully enforceable Landlock ABI 3; enable Landlock in the kernel and permit its syscalls, or run pryx on a host that provides it'
  if (process.getuid?.() === 0 && probe.capabilities['separate-users']?.state !== 'on')
    return 'Embedded PostgreSQL cannot run as root: allow SETUID and SETGID for the container service user, or run this command as an ordinary user'
}
