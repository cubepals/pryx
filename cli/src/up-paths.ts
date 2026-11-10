// SPDX-License-Identifier: FSL-1.1-ALv2
/** Owned installation paths and no-follow preflight. The user preset never writes beyond
 * its home, including when XDG variables or existing links point elsewhere. */
import { existsSync, lstatSync, realpathSync } from 'node:fs'
import { homedir } from 'node:os'
import { isAbsolute, join, relative, resolve, sep } from 'node:path'

export type LocalPaths = {
  home: string
  data: string
  config: string
  state: string
  cache: string
  runtime: string
  cli: string
  bootstrap: string
  unit: string
  client: string
}
export function ownedPath(path: string, home: string, uid = process.getuid?.()) {
  const below = relative(home, resolve(path))
  if (below === '..' || below.startsWith(`..${sep}`) || isAbsolute(below))
    throw new Error(`The user preset writes only below ${home}; invalid path ${path}`)
  let current = home
  for (const part of ['', ...below.split(sep).filter(Boolean)]) {
    current = join(current, part)
    if (!existsSync(current)) {
      // lstat also detects a dangling link, which existsSync intentionally follows.
      try {
        lstatSync(current)
        throw new Error(`Managed path must not be a link: ${current}`)
      } catch (error) {
        if ((error as NodeJS.ErrnoException).code === 'ENOENT') continue
        throw error
      }
    }
    const entry = lstatSync(current)
    if (entry.isSymbolicLink() || entry.uid !== uid || entry.mode & 0o022)
      throw new Error(`Managed path must be owned and not writable by others: ${current}`)
  }
  return resolve(path)
}
export function userPaths(env = process.env, home = realpathSync(homedir())): LocalPaths {
  const base = (name: string, fallback: string) =>
    env[name] && isAbsolute(env[name] as string) ? (env[name] as string) : join(home, fallback)
  const runtime = base('XDG_RUNTIME_DIR', '.cache/pryx-runtime')
  const inHome = relative(home, runtime)
  const paths = {
    home,
    data: join(base('XDG_DATA_HOME', '.local/share'), 'pryx'),
    config: join(base('XDG_CONFIG_HOME', '.config'), 'pryx'),
    state: join(base('XDG_STATE_HOME', '.local/state'), 'pryx'),
    cache: join(base('XDG_CACHE_HOME', '.cache'), 'pryx'),
    runtime: join(
      inHome === '..' || inHome.startsWith(`..${sep}`) || isAbsolute(inHome)
        ? join(home, '.cache/pryx-runtime')
        : runtime,
      'pryx',
    ),
    cli: join(home, '.local/bin/pryx'),
    bootstrap: join(home, '.local/share/pryx-cli'),
    unit: join(base('XDG_CONFIG_HOME', '.config'), 'systemd/user/pryx.service'),
    client: env.PRYX_CONFIG ?? join(base('XDG_CONFIG_HOME', '.config'), 'pryx/client.json'),
  }
  for (const path of Object.values(paths)) ownedPath(path, home)
  return paths
}
export function containerPaths(): LocalPaths {
  return {
    home: '/opt/pryx',
    data: '/opt/pryx/local',
    config: '/etc/pryx-local',
    state: '/var/lib/pryx',
    cache: '/opt/pryx/cache',
    runtime: '/run/pryx-local',
    cli: '/usr/local/bin/pryx',
    bootstrap: '/opt/pryx/cli',
    unit: '/etc/pryx-local/no-user-unit',
    client: process.env.PRYX_CONFIG ?? join(homedir(), '.config/pryx/client.json'),
  }
}
