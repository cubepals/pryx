// SPDX-License-Identifier: FSL-1.1-ALv2
/** Optional user-manager supervision. Files remain under home; logind manages its own
 * explicitly enabled linger state. A refused noninteractive operation falls back to foreground. */
import { mkdirSync } from 'node:fs'
import { basename, dirname, join } from 'node:path'
import { MINUTE_MS, SECOND_MS } from '../../control/src/durations.ts'
import type { HostProbe } from '../../deploy/host-probe.ts'
import { createPrivateFile } from '../../deploy/image-state.ts'
import { readOwnedText } from './up-local-record.ts'
import type { LocalPaths } from './up-paths.ts'

const MARKER = '# Managed pryx user installation'
const quote = (value: string, executable = false) =>
  JSON.stringify(value.replaceAll('%', '%%').replaceAll('$', executable ? '$$' : '$'))
export function userUnit(paths: LocalPaths) {
  const environment: Record<string, string> = {
    HOME: paths.home,
    XDG_DATA_HOME: dirname(paths.data),
    XDG_CONFIG_HOME: dirname(paths.config),
    XDG_STATE_HOME: dirname(paths.state),
    XDG_CACHE_HOME: dirname(paths.cache),
    PRYX_CONFIG: paths.client,
    ...(process.env.XDG_RUNTIME_DIR ? { XDG_RUNTIME_DIR: process.env.XDG_RUNTIME_DIR } : {}),
  }
  return `${MARKER}
[Unit]
Description=pryx personal sandbox service
[Service]
ExecStart=${quote(join(paths.bootstrap, 'bin/pryx'), true)} up --preset user --run-installed
WorkingDirectory=${quote(paths.home)}
${Object.entries(environment)
  .map(([key, value]) => `Environment=${quote(`${key}=${value}`)}`)
  .join('\n')}
Restart=on-failure
RestartSec=${(2 * SECOND_MS) / SECOND_MS}
KillMode=process
TimeoutStopSec=${MINUTE_MS / SECOND_MS / 2}
NoNewPrivileges=yes
[Install]
WantedBy=default.target
`
}
async function command(args: string[]) {
  const child = Bun.spawn(args, { stdin: 'ignore', stdout: 'ignore', stderr: 'pipe' })
  const timer = setTimeout(() => child.kill('SIGKILL'), MINUTE_MS / 2)
  try {
    const [stderr, code] = await Promise.all([new Response(child.stderr).text(), child.exited])
    return { ok: code === 0, detail: stderr.trim().slice(-512) }
  } finally {
    clearTimeout(timer)
  }
}
export async function startUserUnit(paths: LocalPaths, probe: HostProbe, previousLinger = false) {
  if (probe.capabilities['user-session']?.state !== 'on' || !Bun.which('systemctl'))
    return {
      enabled: false,
      enabledLinger: previousLinger,
      action: 'No usable systemd user session; keep the foreground supervisor running',
    }
  const session = await command(['systemctl', '--user', '--no-ask-password', 'show-environment'])
  if (!session.ok)
    return {
      enabled: false,
      enabledLinger: previousLinger,
      action: 'The systemd user manager did not answer; keep the foreground supervisor running',
    }
  try {
    const existing = readOwnedText(paths.unit)
    if (existing !== undefined && !existing.startsWith(MARKER))
      return {
        enabled: false,
        enabledLinger: previousLinger,
        action: `Existing unit ${paths.unit} is not managed by pryx; keep the foreground supervisor running`,
      }
  } catch {
    return {
      enabled: false,
      enabledLinger: previousLinger,
      action: `User unit path ${paths.unit} is unavailable; keep the foreground supervisor running`,
    }
  }
  let enabledLinger = previousLinger
  if (probe.capabilities.linger?.state !== 'on') {
    const linger = await command(['loginctl', '--no-ask-password', 'enable-linger'])
    if (!linger.ok)
      return {
        enabled: false,
        enabledLinger,
        action:
          'Linger was refused; an administrator can run loginctl enable-linger for this user. Keep the foreground supervisor running',
      }
    enabledLinger = true
  }
  // createPrivateFile renames atomically and never follows a destination link.
  mkdirSync(dirname(paths.unit), { recursive: true, mode: 0o700 })
  createPrivateFile(paths.unit, userUnit(paths), true)
  const reload = await command(['systemctl', '--user', '--no-ask-password', 'daemon-reload'])
  const started =
    reload.ok &&
    (await command(['systemctl', '--user', '--no-ask-password', 'enable', '--now', paths.unit])).ok
  if (!started)
    return {
      enabled: false,
      enabledLinger,
      action:
        'The user unit could not start; inspect systemctl --user status pryx. Keep the foreground supervisor running',
    }
  return { enabled: true, enabledLinger, action: 'systemctl --user status pryx' }
}
export async function stopUserUnit(paths: LocalPaths, removeLinger: boolean) {
  const stopped = await command([
    'systemctl',
    '--user',
    '--no-ask-password',
    'disable',
    '--now',
    basename(paths.unit),
  ])
  if (!stopped.ok) throw new Error('Could not stop the pryx user unit; inspect systemctl --user status pryx')
  if (removeLinger) {
    const linger = await command(['loginctl', '--no-ask-password', 'disable-linger'])
    if (!linger.ok) throw new Error('Could not remove pryx-enabled linger; inspect loginctl before purging')
  }
}
