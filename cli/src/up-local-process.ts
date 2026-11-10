// SPDX-License-Identifier: FSL-1.1-ALv2
/** Launch and stop a managed supervisor. Readiness needs its owned identity record after
 * gateway binding, so an unrelated listener cannot impersonate a successful startup. */
import { join } from 'node:path'
import { MINUTE_MS, SECOND_MS } from '../../control/src/durations.ts'
import { HTTP_FRONT_PORT } from '../../control/src/edge-headers.ts'
import { PARENT_DEATH_SIGNAL } from '../../deploy/image-state.ts'
import { readClientConfig } from './client-config.ts'
import { localRequirements } from './up-local-policy.ts'
import { type LocalInstallation, supervisor } from './up-local-record.ts'

export function localEnvironment(record: LocalInstallation): NodeJS.ProcessEnv {
  return {
    ...process.env,
    PRYX_BUNDLE_DIR: record.bundle,
    PRYX_ASSET_DIR: join(record.bundle, 'control'),
    PRYX_PRYXD_BIN: join(record.bundle, 'bin/pryxd'),
    PRYX_AGENTOS_DISTRIBUTION: join(record.bundle, 'agentos'),
    PRYX_LOCAL_PRESET: record.preset,
    PRYX_LOCAL_LAUNCHER: join(record.paths.bootstrap, 'bin/pryxd'),
    PRYX_STATE_DIR: record.paths.state,
    PRYX_LOCAL_CONFIG_DIR: record.paths.config,
    PRYX_RUNTIME_DIR: record.paths.runtime,
    PRYX_CLIENT_CONFIG: record.clientConfig,
    PRYX_PUBLIC_URL: record.apiUrl,
    PRYX_DEFAULT_TIER: record.tier,
    PRYX_REQUIRE: localRequirements(record.require).join(','),
    PORT: new URL(record.apiUrl).port || String(HTTP_FRONT_PORT),
    TMPDIR: record.paths.runtime,
    PATH: `${join(record.bundle, 'bin')}:${process.env.PATH ?? ''}`,
  }
}
export function launchLocal(record: LocalInstallation) {
  // util-linux flock --no-fork holds the lock across exec, including the bundled loader.
  return Bun.spawn(
    [
      join(record.bundle, 'bin/flock'),
      '-n',
      '--no-fork',
      join(record.paths.runtime, 'supervisor.lock'),
      join(record.bundle, 'bin/tini'),
      '-s',
      '-g',
      '-p',
      PARENT_DEATH_SIGNAL,
      '--',
      join(record.bundle, 'bin/pryx-image'),
    ],
    { env: localEnvironment(record), stdin: 'ignore', stdout: 'inherit', stderr: 'inherit' },
  )
}
export async function localReady(record: LocalInstallation, child?: Bun.Subprocess) {
  const deadline = performance.now() + 2 * MINUTE_MS
  while (performance.now() < deadline) {
    if (child && child.exitCode !== null)
      throw new Error(`pryx stopped during startup; inspect ${join(record.paths.config, 'startup.log')}`)
    const identity = supervisor(record)
    if (
      identity?.ready &&
      readClientConfig(record.clientConfig) &&
      (
        await fetch(`${record.apiUrl}/healthz`, { signal: AbortSignal.timeout(2 * SECOND_MS) }).catch(
          () => undefined,
        )
      )?.ok
    )
      return
    await Bun.sleep(SECOND_MS / 10)
  }
  throw new Error(`pryx did not become ready; inspect ${join(record.paths.config, 'startup.log')}`)
}
export async function followLocal(child: Bun.Subprocess) {
  const stop = () => {
    if (child.exitCode === null) child.kill('SIGTERM')
  }
  process.once('SIGINT', stop)
  process.once('SIGTERM', stop)
  try {
    const code = await child.exited
    if (code) throw new Error('pryx supervisor stopped; inspect its private startup log')
  } finally {
    process.removeListener('SIGINT', stop)
    process.removeListener('SIGTERM', stop)
  }
}
export async function stopLocal(record: LocalInstallation) {
  const identity = supervisor(record)
  if (!identity) return
  process.kill(identity.pid, 'SIGTERM')
  const deadline = performance.now() + MINUTE_MS / 2
  while (performance.now() < deadline) {
    const current = supervisor(record)
    if (!current || current.start !== identity.start || current.pid !== identity.pid) return
    await Bun.sleep(SECOND_MS / 10)
  }
  const current = supervisor(record)
  if (current?.start === identity.start && current.pid === identity.pid) process.kill(identity.pid, 'SIGKILL')
}
