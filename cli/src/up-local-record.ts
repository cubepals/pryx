// SPDX-License-Identifier: FSL-1.1-ALv2
/** Owner-only installation records and process identity. No deletion or signalling occurs
 * until a managed process and its original start time have both been verified. */
import { closeSync, constants, fstatSync, openSync, readFileSync } from 'node:fs'
import { isAbsolute, join, relative, sep } from 'node:path'
import { GUARANTEES, type Guarantee } from '../../control/src/guarantees.ts'
import { createPrivateFile } from '../../deploy/image-state.ts'
import { processStart } from '../../deploy/supervisor-process.ts'
import { type ProfileRecord, validProfile } from './up-local-profile.ts'
import type { LocalPaths } from './up-paths.ts'

export type LocalInstallation = {
  version: string
  digest: string
  bundle: string
  preset: 'user' | 'container'
  tier: 'process' | 'wasm' | 'container'
  runtime?: 'auto'
  apiUrl: string
  clientConfig: string
  paths: LocalPaths
  userUnit: boolean
  enabledLinger: boolean
  profiles?: ProfileRecord[]
  require?: Guarantee[]
}
export function readOwnedText(path: string): string | undefined {
  let fd: number
  try {
    fd = openSync(path, constants.O_RDONLY | constants.O_NOFOLLOW | constants.O_NONBLOCK)
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === 'ENOENT') return undefined
    throw error
  }
  try {
    const info = fstatSync(fd)
    if (!info.isFile() || info.uid !== process.getuid?.() || info.mode & 0o077 || info.size > 64 * 1024)
      throw new Error('Managed records must be owner-only regular files')
    return readFileSync(fd, 'utf8')
  } finally {
    closeSync(fd)
  }
}
export function readOwnedJson<T>(path: string): T | undefined {
  const text = readOwnedText(path)
  return text === undefined ? undefined : (JSON.parse(text) as T)
}
export function readInstallation(paths: LocalPaths) {
  const record = readOwnedJson<LocalInstallation>(join(paths.config, 'installation.json'))
  if (!record) return
  const below =
    typeof record.bundle === 'string' ? relative(join(paths.data, 'releases'), record.bundle) : '..'
  if (
    !below ||
    below === '..' ||
    below.startsWith(`..${sep}`) ||
    isAbsolute(below) ||
    !['user', 'container'].includes(record.preset) ||
    !['process', 'wasm', 'container'].includes(record.tier) ||
    (record.runtime !== undefined && record.runtime !== 'auto') ||
    !/^[a-f0-9]{64}$/.test(record.digest) ||
    !/^\d+\.\d+\.\d+$/.test(record.version) ||
    JSON.stringify(record.paths) !== JSON.stringify(paths) ||
    record.clientConfig !== paths.client ||
    (record.require !== undefined &&
      (!Array.isArray(record.require) || !record.require.every((name) => GUARANTEES.includes(name)))) ||
    (record.profiles !== undefined &&
      (!Array.isArray(record.profiles) ||
        !record.profiles.every((profile) => validProfile(profile, paths.home))))
  )
    throw new Error('Invalid managed installation; keep its state and inspect installation.json')
  return record
}
export function writeInstallation(record: LocalInstallation) {
  createPrivateFile(join(record.paths.config, 'installation.json'), `${JSON.stringify(record)}\n`, true)
}
export function supervisor(record: LocalInstallation) {
  const identity = readOwnedJson<{
    pid: number
    start: string
    bundle: string
    state: string
    ready: boolean
  }>(join(record.paths.runtime, 'supervisor.json'))
  if (
    !identity ||
    !Number.isSafeInteger(identity.pid) ||
    identity.pid <= 0 ||
    identity.start !== processStart(identity.pid) ||
    identity.bundle !== record.bundle ||
    identity.state !== record.paths.state
  )
    return
  let argv: string[]
  try {
    argv = readFileSync(`/proc/${identity.pid}/cmdline`, 'utf8').split('\0')
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === 'ENOENT') return
    throw error
  }
  if (!argv.includes(join(record.bundle, 'bin/pryx-image.real')))
    throw new Error('Supervisor process identity differs; no signal was sent')
  return identity
}
