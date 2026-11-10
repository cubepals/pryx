// SPDX-License-Identifier: FSL-1.1-ALv2
/** Stop a managed local installation before optional deletion. Runtime cleanup is delegated
 * to the node backend while its exclusive store lock prevents a concurrent daemon restart. */
import { existsSync, readFileSync, rmSync } from 'node:fs'
import { join } from 'node:path'
import { option } from './sandbox.ts'
import { localPaths } from './up-local.ts'
import { acquireLifecycle } from './up-local-lock.ts'
import { localEnvironment, stopLocal } from './up-local-process.ts'
import { readInstallation, readOwnedText } from './up-local-record.ts'
import { selectPreset } from './up-preset.ts'
import { disableUserLinger, stopUserUnit } from './up-user-unit.ts'

export async function down(args: string[]) {
  const preset = selectPreset(option(args, '--preset'))
  if (preset !== 'user' && preset !== 'container')
    throw new Error(
      'For a server, stop its documented systemd units; automatic purge serves user and container presets',
    )
  const paths = localPaths(preset)
  if (!existsSync(join(paths.config, 'installation.json'))) return { stopped: true, removed: [] }
  const unlock = acquireLifecycle(paths.operation)
  const removed: string[] = []
  try {
    const record = readInstallation(paths)
    if (!record) return { stopped: true, removed }
    const purge = args.includes('--purge')
    if (record.userUnit) await stopUserUnit(paths, purge && record.enabledLinger)
    else if (purge && record.enabledLinger) await disableUserLinger()
    await stopLocal(record)
    const config = join(paths.config, 'pryxd.toml')
    if (existsSync(config)) {
      const child = Bun.spawn(
        [join(record.bundle, 'bin/pryxd'), 'local-stop', '--config', config, ...(purge ? ['--purge'] : [])],
        { env: localEnvironment(record), stdin: 'ignore', stdout: 'ignore', stderr: 'pipe' },
      )
      const detail = await new Response(child.stderr).text()
      if (await child.exited)
        throw new Error(`Compute cleanup refused; state was kept: ${detail.slice(-2048)}`)
    }
    if (!purge) return { stopped: true, state: paths.state, removed }
    const shortcut = existsSync(paths.cli) ? readFileSync(paths.cli, 'utf8') : ''
    if (shortcut && !shortcut.includes('# Managed pryx installation'))
      throw new Error('CLI shortcut was replaced; no installation files were removed')
    const unit = readOwnedText(paths.unit)
    if (unit !== undefined && !unit.startsWith('# Managed pryx user installation'))
      throw new Error('User unit was replaced; no installation files were removed')
    for (const path of [
      ...new Set([
        paths.unit,
        paths.client,
        paths.cli,
        paths.runtime,
        paths.cache,
        paths.state,
        paths.config,
        paths.data,
        paths.bootstrap,
      ]),
    ]) {
      if (existsSync(path)) {
        rmSync(path, { recursive: true, force: true })
        removed.push(path)
      }
    }
    // Last mutation while still holding the old inode: no replacement lock can race deletion.
    rmSync(paths.operation)
    removed.push(paths.operation)
    return { stopped: true, removed }
  } finally {
    unlock()
  }
}
