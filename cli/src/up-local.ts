// SPDX-License-Identifier: FSL-1.1-ALv2
/** Install or resume the shared supervisor on a host. Release staging precedes activation;
 * the short lifecycle transaction never spans a foreground supervisor's lifetime. */
import { existsSync, mkdirSync, readFileSync, rmSync } from 'node:fs'
import { join } from 'node:path'
import { selectedRelease } from './bundle-download.ts'
import { type BundleMetadata, retainBundle, stageBundle } from './bundle-install.ts'
import { option } from './sandbox.ts'
import { activateCli, localDirectories, localSummary } from './up-local-activate.ts'
import { inspectLocal } from './up-local-check.ts'
import { acquireLifecycle } from './up-local-lock.ts'
import { configuredPort, localPort } from './up-local-network.ts'
import { followLocal, launchLocal, localReady, stopLocal } from './up-local-process.ts'
import { type LocalInstallation, readInstallation, supervisor, writeInstallation } from './up-local-record.ts'
import { containerPaths, userPaths } from './up-paths.ts'
import type { Preset } from './up-preset.ts'
import { prepareUserUnit, startUserUnit, stopUserUnit } from './up-user-unit.ts'

export function localPaths(preset: Preset) {
  return preset === 'user' ? userPaths() : containerPaths()
}
export async function runInstalled(preset: Preset) {
  const paths = localPaths(preset)
  if (!existsSync(join(paths.config, 'installation.json')))
    throw new Error('No managed installation; run pryx up')
  const unlock = acquireLifecycle(paths.operation)
  let child: Bun.Subprocess | undefined
  try {
    const record = readInstallation(paths)
    if (!record) throw new Error('No managed installation; run pryx up')
    if (supervisor(record)) throw new Error('pryx is already running')
    localDirectories(paths)
    child = launchLocal(record)
    await localReady(record, child)
  } catch (error) {
    child?.kill('SIGTERM')
    throw error
  } finally {
    unlock()
  }
  await followLocal(child)
}
export async function upLocal(args: string[], preset: 'user' | 'container', upgrading = false) {
  const paths = localPaths(preset)
  const port = configuredPort()
  const initial = await inspectLocal(preset)
  // This probe and all path checks finish before the first persistent mutation.
  mkdirSync(paths.home, { recursive: true, mode: 0o755 })
  const unlock = acquireLifecycle(paths.operation)
  let child: Bun.Subprocess | undefined
  let record: LocalInstallation | undefined
  let action = 'Keep this foreground supervisor running; use a terminal multiplexer to keep it after logout'
  let startUnit = false
  try {
    const previous = readInstallation(paths)
    if (upgrading && !previous) throw new Error('No managed installation; run pryx up first')
    if (previous && !upgrading) record = previous
    else {
      localDirectories(paths)
      const source = option(args, '--from-dir')
      const selected = await selectedRelease(upgrading, source)
      const staged = await stageBundle(paths, { source, ...selected })
      try {
        const checked = await inspectLocal(preset, staged.unpacked, staged.temporary)
        if (previous) {
          const old = JSON.parse(readFileSync(join(previous.bundle, 'bundle.json'), 'utf8')) as BundleMetadata
          if (old.postgres.major !== staged.metadata.postgres.major)
            throw new Error(
              'PostgreSQL major upgrades require an explicit database migration; current state was kept',
            )
        }
        const bundle = retainBundle(paths, staged)
        if (previous?.userUnit) await stopUserUnit(paths, false)
        if (previous) await stopLocal(previous)
        record = {
          version: staged.metadata.version,
          digest: staged.digest,
          bundle,
          preset,
          tier: checked.tier,
          apiUrl: previous?.apiUrl ?? `http://127.0.0.1:${await localPort(port)}`,
          clientConfig: paths.client,
          paths,
          userUnit: false,
          enabledLinger: previous?.enabledLinger ?? false,
        }
      } finally {
        rmSync(staged.temporary, { recursive: true, force: true })
      }
    }
    activateCli(record)
    if (supervisor(record)?.ready) {
      await localReady(record)
      await localSummary(record, record.userUnit ? 'systemctl --user status pryx' : action)
      return
    }
    writeInstallation(record)
    if (preset === 'user') {
      const current = record
      const unit = await prepareUserUnit(paths, initial.probe, record.enabledLinger, () => {
        current.enabledLinger = true
        writeInstallation(current)
      })
      record.userUnit = unit.enabled
      record.enabledLinger = unit.enabledLinger
      action = unit.action
      startUnit = unit.enabled
    }
    writeInstallation(record)
    if (!startUnit) {
      child = launchLocal(record)
      await localReady(record, child)
      await localSummary(record, action)
    }
  } catch (error) {
    child?.kill('SIGTERM')
    throw error
  } finally {
    unlock()
  }
  if (startUnit && record) {
    if (!(await startUserUnit(paths)))
      throw new Error('Could not start the user unit; inspect systemctl --user status pryx')
    await localReady(record)
    await localSummary(record, action)
  }
  if (child) await followLocal(child)
}
