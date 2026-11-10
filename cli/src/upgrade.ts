// SPDX-License-Identifier: FSL-1.1-ALv2
/** Upgrade an authenticated local bundle while retaining database and sandbox state. */
import { option } from './sandbox.ts'
import { upLocal } from './up-local.ts'
import { selectPreset } from './up-preset.ts'

export async function upgrade(args: string[]) {
  const preset = selectPreset(option(args, '--preset'))
  if (preset !== 'user' && preset !== 'container')
    throw new Error(
      'For a server upgrade, run pryx up --preset server; local bundle upgrades serve user and container presets',
    )
  return await upLocal(args, preset, true)
}
