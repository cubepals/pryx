// SPDX-License-Identifier: FSL-1.1-ALv2
/** Install or resume the shared supervisor on a host. Release staging precedes activation;
 * the short lifecycle transaction never spans a foreground supervisor's lifetime. */
import { existsSync, mkdirSync, readFileSync, rmSync } from 'node:fs'
import { join } from 'node:path'
import { HTTP_FRONT_PORT } from '../../control/src/edge-headers.ts'
import { selectedRelease } from './bundle-download.ts'
import { type BundleMetadata, retainBundle, stageBundle } from './bundle-install.ts'
import { option } from './sandbox.ts'
import { activateCli, localDirectories, localSummary } from './up-local-activate.ts'
import { inspectLocal } from './up-local-check.ts'
import { acquireLifecycle } from './up-local-lock.ts'
import { configuredPort, localPort } from './up-local-network.ts'
import { localRequirements } from './up-local-policy.ts'
import { followLocal, launchLocal, localReady, stopLocal } from './up-local-process.ts'
import { installProfiles } from './up-local-profile.ts'
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
  localRequirements()
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
    if (previous && port !== undefined && port !== Number(new URL(previous.apiUrl).port || HTTP_FRONT_PORT))
      throw new Error('The existing gateway port is immutable; migrate public API origins before changing it')
    const require = localRequirements(previous?.require)
    if (previous && process.env.PRYX_DEFAULT_TIER && previous.tier !== process.env.PRYX_DEFAULT_TIER)
      throw new Error(
        `The existing ${previous.tier} backend owns sandbox state; migrate its data before changing backends`,
      )
    if (upgrading && !previous) throw new Error('No managed installation; run pryx up first')
    if (previous && !upgrading) {
      record = { ...previous, require }
      if (JSON.stringify(previous.require ?? []) !== JSON.stringify(require)) {
        if (previous.userUnit) await stopUserUnit(paths, false)
        await stopLocal(previous)
      }
    } else {
      localDirectories(paths)
      const source = option(args, '--from-dir')
      const selected = await selectedRelease(upgrading, source)
      const staged = await stageBundle(paths, { source, ...selected })
      try {
        const checked = await inspectLocal(
          preset,
          staged.unpacked,
          staged.temporary,
          previous?.tier ?? process.env.PRYX_DEFAULT_TIER,
        )
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
          runtime:
            previous?.runtime ??
            (!previous && !process.env.PRYX_DEFAULT_TIER && checked.probe.tiers ? 'auto' : undefined),
          apiUrl: previous?.apiUrl ?? `http://127.0.0.1:${await localPort(port)}`,
          clientConfig: paths.client,
          paths,
          userUnit: false,
          enabledLinger: previous?.enabledLinger ?? false,
          profiles: previous?.profiles,
          require,
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
      record.profiles = installProfiles(paths, record.profiles)
      writeInstallation(record)
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
    await startUserUnit(paths)
    await localReady(record)
    await localSummary(record, action)
  }
  if (child) await followLocal(child)
}
