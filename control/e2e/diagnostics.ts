// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Failure evidence excludes container environments, commands and application output. */
import { spawnSync } from 'node:child_process'
import { readdirSync, readFileSync } from 'node:fs'
import { homedir } from 'node:os'
import { dirname, join } from 'node:path'
import type { deployment } from './deployment.ts'

export function deploymentDiagnostics(d: Awaited<ReturnType<typeof deployment>>) {
  for (const { path } of d.children) {
    const log = readFileSync(join(dirname(path), 'pryxd.log'), 'utf8')
    console.error(
      'NODE_REQUEST_STATUS',
      log
        .split('\n')
        .filter((line) => /status=/.test(line))
        .slice(-40)
        .join('\n'),
    )
  }
  const listed = spawnSync('docker', [
    'ps',
    '-aq',
    '--filter',
    `label=blocklyd.deployment=${d.control.config.deployment}`,
  ])
    .stdout.toString()
    .split(/\s+/)
    .filter(Boolean)
  for (const id of listed) {
    const state = spawnSync('docker', [
      'inspect',
      '--format',
      '{{json .State.Status}} {{json .State.ExitCode}}',
      id,
    ])
    console.error('CONTAINER_STATE', id, state.stdout.toString().trim())
  }
}

export function rivetDiagnostics() {
  const directory = join(homedir(), '.rivetkit/var/logs/rivet-engine')
  try {
    const secrets = Object.entries(process.env)
      .filter(
        ([key, value]) => /TOKEN|KEY|SECRET|PASSWORD|DATABASE_URL/i.test(key) && value && value.length >= 8,
      )
      .map(([, value]) => value as string)
    for (const name of readdirSync(directory)
      .filter((name) => name.endsWith('-stderr.log'))
      .slice(-2)) {
      let lines = readFileSync(join(directory, name), 'utf8')
        .split('\n')
        .filter((line) => /error|panic|failed/i.test(line))
        .slice(-30)
        .join('\n')
      for (const secret of secrets) lines = lines.replaceAll(secret, '[redacted]')
      console.error('RIVET_ENGINE_FAILURE', lines)
    }
  } catch (error) {
    console.error('RIVET_ENGINE_DIAGNOSTICS', error instanceof Error ? error.name : 'Unavailable')
  }
}
