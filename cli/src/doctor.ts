// SPDX-License-Identifier: FSL-1.1-ALv2
/** Display measured tiers with the caller's own managed configuration when it can be found.
 * The daemon owns probing and formatting; this command never starts services or changes state. */
import { existsSync, lstatSync } from 'node:fs'
import { homedir } from 'node:os'
import { dirname, join } from 'node:path'
import { SECOND_MS } from '../../control/src/durations.ts'
import { IMAGE_NODE_CONFIG } from '../../deploy/image-state.ts'
import { privateDirectory, stateRoot } from '../../deploy/local-layout.ts'
import { containerPaths } from './up-paths.ts'

function configuredNode() {
  const candidates = [
    process.env.PRYX_LOCAL_CONFIG_DIR && join(process.env.PRYX_LOCAL_CONFIG_DIR, IMAGE_NODE_CONFIG),
    process.env.PRYX_STATE_DIR &&
      join(
        privateDirectory(stateRoot(process.env.PRYX_STATE_DIR, Boolean(process.env.PRYX_LOCAL_PRESET))),
        IMAGE_NODE_CONFIG,
      ),
    join(process.env.XDG_CONFIG_HOME ?? join(homedir(), '.config'), 'pryx', IMAGE_NODE_CONFIG),
    // pryxd's documented system configuration, when it belongs to this caller.
    '/etc/pryxd/pryxd.toml',
  ]
  if (process.getuid?.() === 0) {
    try {
      candidates.unshift(join(containerPaths().config, IMAGE_NODE_CONFIG))
    } catch {}
  }
  return candidates.find((path) => {
    if (!path) return false
    try {
      const file = lstatSync(path),
        parent = lstatSync(dirname(path))
      return (
        file.isFile() &&
        !file.isSymbolicLink() &&
        file.uid === process.getuid?.() &&
        !(file.mode & 0o022) &&
        parent.isDirectory() &&
        !parent.isSymbolicLink() &&
        parent.uid === process.getuid?.() &&
        !(parent.mode & 0o022)
      )
    } catch {
      return false
    }
  })
}

export async function doctor(args: string[]) {
  if (args.some((argument) => argument !== '--json')) throw new Error('usage: pryx doctor [--json]')
  const binary =
    process.env.PRYX_PRYXD_BIN ??
    Bun.which('pryxd') ??
    ['/opt/pryx/pryxd', '/usr/local/bin/pryxd'].find((path) => existsSync(path))
  if (!binary) throw new Error('pryxd is missing; install pryx on this host before running pryx doctor')
  const config = configuredNode()
  const help = config
    ? Bun.spawnSync([binary, 'probe', '--help'], { stdout: 'pipe', stderr: 'ignore', timeout: 5 * SECOND_MS })
    : undefined
  const configured = config && help?.exitCode === 0 && help.stdout.toString().includes('--config')
  const child = Bun.spawn(
    [
      binary,
      'probe',
      ...(args.includes('--json') ? ['--json'] : []),
      ...(configured ? ['--config', config] : []),
    ],
    {
      stdin: 'ignore',
      stdout: 'inherit',
      stderr: 'inherit',
    },
  )
  if (await child.exited) throw new Error('The host probe failed; inspect the reported requirement')
}
