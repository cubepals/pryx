// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Real daemon process lifetime and the systemctl join fixture. Database and endpoint setup remain in the fleet suite. */
import { spawnSync } from 'node:child_process'
import { openSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { MINUTE_MS } from '../src/durations.ts'

const env = process.env
export const BIN = env.PRYXD_BIN ?? new URL('../../target/debug/pryxd', import.meta.url).pathname
export const SUDO = env.PRYXD_SUDO === '1'

export interface Daemon {
  config: string
  log: string
  opsPort: number
  start: () => void
  kill: (signal: 'KILL' | 'TERM') => void
  /** `pryxd join` on this node, with a stand-in for systemctl that stops and starts it. */
  join: (token: string) => ReturnType<typeof spawnSync>
}

/** pryxd serving `config` as a process of its own, as root, with its log in `home`. */
export function daemon(home: string, config: string, opsPort: number): Daemon {
  const log = join(home, 'pryxd.log')
  return {
    config,
    log,
    opsPort,
    start: () => {
      const out = openSync(log, 'a')
      const command = [BIN ?? '', 'serve', '--config', config, '--log-format', 'pretty']
      Bun.spawn(SUDO ? ['sudo', '-n', ...command] : command, {
        stdout: out,
        stderr: out,
      })
    },
    kill: (signal) => {
      const pkill = ['pkill', `-${signal}`, '-f', '--', `--config ${config}`]
      spawnSync(SUDO ? 'sudo' : (pkill[0] ?? 'pkill'), SUDO ? ['-n', ...pkill] : pkill.slice(1))
    },
    join: (token) => {
      // `serve --config`, so the pattern never matches the join that runs this.
      const serving = `serve --config ${config}`
      const systemctl = join(home, 'systemctl')
      writeFileSync(
        systemctl,
        [
          '#!/bin/sh',
          'case "$1" in',
          `stop) pkill -TERM -f -- '${serving}'; while pgrep -f -- '${serving}' >/dev/null; do sleep 0.2; done ;;`,
          `start|restart) nohup '${BIN}' ${serving} --log-format pretty >>'${log}' 2>&1 & ;;`,
          'esac',
          '',
        ].join('\n'),
        { mode: 0o755 },
      )
      const command = [BIN ?? '', 'join', token, '--config', config, '--systemctl', systemctl]
      return spawnSync(SUDO ? 'sudo' : (command[0] ?? ''), SUDO ? ['-n', ...command] : command.slice(1), {
        encoding: 'utf8',
        timeout: 3 * MINUTE_MS,
      })
    },
  }
}
