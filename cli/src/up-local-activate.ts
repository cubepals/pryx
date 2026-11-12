// SPDX-License-Identifier: FSL-1.1-ALv2
/** Activate only an authenticated release. Bootstrap shortcuts are atomic files pointing at
 * immutable bundles; sandbox state and older releases remain available across upgrades. */
import { chmodSync, existsSync, mkdirSync, readFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { containerContext } from '../../deploy/host-context.ts'
import { capabilitySummary, hostProbe } from '../../deploy/host-probe.ts'
import { createPrivateFile, IMAGE_NODE_CONFIG } from '../../deploy/image-state.ts'
import { readClientConfig } from './client-config.ts'
import { localEnvironment } from './up-local-process.ts'
import type { LocalInstallation } from './up-local-record.ts'
import type { LocalPaths } from './up-paths.ts'

const shellQuote = (value: string) => `'${value.replaceAll("'", "'\\''")}'`
export function localDirectories(paths: LocalPaths) {
  for (const path of [paths.data, paths.config, paths.state, paths.cache, paths.runtime]) {
    const mode =
      process.getuid?.() === 0 && path === paths.data
        ? 0o755
        : process.getuid?.() === 0 && path === paths.runtime
          ? 0o711
          : 0o700
    mkdirSync(path, { recursive: true, mode })
    if (process.getuid?.() === 0) chmodSync(path, mode)
  }
}
export function activateCli(record: LocalInstallation) {
  const executable = join(record.paths.bootstrap, 'bin/pryx')
  const launcher = join(record.paths.bootstrap, 'bin/pryxd')
  mkdirSync(dirname(launcher), { recursive: true, mode: 0o700 })
  createPrivateFile(launcher, readFileSync(join(record.bundle, 'bin/pryxd')), true, 0o755)
  const target = join(record.bundle, 'bin/pryx')
  const text = `#!/bin/sh\n# Managed pryx installation\nexec ${shellQuote(target)} "$@"\n`
  for (const path of [executable, record.paths.cli]) {
    mkdirSync(dirname(path), { recursive: true, mode: 0o700 })
    createPrivateFile(path, text, true, 0o755)
  }
}
export async function localSummary(record: LocalInstallation, action: string) {
  const saved = readClientConfig(record.clientConfig)
  if (!saved) throw new Error('The service did not save its private CLI credentials')
  const configured = join(record.paths.config, IMAGE_NODE_CONFIG)
  const probe = await hostProbe(
    join(record.bundle, 'bin/pryxd'),
    localEnvironment(record),
    existsSync(configured) ? configured : undefined,
  )
  const printed = join(record.paths.config, 'key-printed')
  let firstKey = false
  try {
    createPrivateFile(printed, 'printed\n')
    firstKey = true
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== 'EEXIST') throw error
  }
  console.log(
    JSON.stringify(
      {
        version: record.version,
        preset: record.preset,
        apiUrl: record.apiUrl,
        tiers:
          record.runtime === 'auto'
            ? Object.entries(probe.tiers ?? {})
                .filter(
                  ([tier, value]) => value.state === 'on' && (tier !== 'container' || !containerContext()),
                )
                .map(([tier]) => tier)
            : [record.tier],
        require: record.require ?? [],
        ...(record.defaultIsolation ? { defaultIsolation: record.defaultIsolation } : {}),
        capabilities: capabilitySummary(probe),
        credentialPath: record.clientConfig,
        supervision: action,
        ...(firstKey ? { apiKey: saved.key } : {}),
      },
      null,
      2,
    ),
  )
  if (record.preset === 'user')
    console.log(
      "Process sandboxes share your user and have no enforced resource limits; use them for one person's work, not strangers' code.",
    )
  console.log(`${shellQuote(record.paths.cli)} run -- echo hello`)
}
