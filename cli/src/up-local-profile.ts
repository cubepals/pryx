// SPDX-License-Identifier: FSL-1.1-ALv2
/** Add the user CLI to future POSIX shell sessions. Only an exact owned block is removed
 * on purge; linked, unsafe or read-only startup files are never changed by installation. */
import { closeSync, constants, existsSync, fstatSync, openSync, readFileSync, unlinkSync } from 'node:fs'
import { basename, join } from 'node:path'
import { createPrivateFile } from '../../deploy/image-state.ts'
import type { LocalPaths } from './up-paths.ts'
import { ownedPath } from './up-paths.ts'

export type ProfileRecord = { path: string; created: boolean }
const NAMES = ['.profile', '.bashrc', '.bash_profile', '.zprofile', '.zshrc']
const BLOCK = `
# >>> pryx user PATH >>>
case ":$PATH:" in
  *":$HOME/.local/bin:"*) ;;
  *) PATH="$HOME/.local/bin:$PATH"; export PATH ;;
esac
# <<< pryx user PATH <<<
`
function read(path: string) {
  let fd: number
  try {
    fd = openSync(path, constants.O_RDONLY | constants.O_NOFOLLOW | constants.O_NONBLOCK)
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === 'ENOENT') return { text: '', mode: 0o600, created: true }
    throw error
  }
  try {
    const info = fstatSync(fd)
    if (
      !info.isFile() ||
      info.nlink !== 1 ||
      info.uid !== process.getuid?.() ||
      info.mode & 0o022 ||
      !(info.mode & 0o200) ||
      info.size > 1024 * 1024
    )
      throw new Error('Shell profile is not an owned writable regular file')
    return { text: readFileSync(fd, 'utf8'), mode: info.mode & 0o777, created: false }
  } finally {
    closeSync(fd)
  }
}
export function validProfile(profile: ProfileRecord, home: string) {
  if (!profile || typeof profile !== 'object') return false
  return typeof profile.created === 'boolean' && NAMES.some((name) => profile.path === join(home, name))
}
export function installProfiles(
  paths: LocalPaths,
  previous: ProfileRecord[] = [],
  shell = process.env.SHELL,
) {
  const profiles = [...previous]
  const name = basename(shell ?? 'sh')
  const names =
    name === 'zsh' ? ['.zprofile', '.zshrc'] : name === 'bash' ? ['.profile', '.bashrc'] : ['.profile']
  if (name === 'bash' && existsSync(join(paths.home, '.bash_profile'))) names.push('.bash_profile')
  for (const name of names) {
    const path = join(paths.home, name)
    let saved: ReturnType<typeof read>
    try {
      ownedPath(path, paths.home)
      saved = read(path)
    } catch {
      // PATH integration is optional for a linked/custom profile; the printed absolute CLI still works.
      continue
    }
    if (!saved.text.includes(BLOCK)) createPrivateFile(path, saved.text + BLOCK, true, saved.mode)
    if (!profiles.some((profile) => profile.path === path)) profiles.push({ path, created: saved.created })
  }
  return profiles
}
export function removeProfiles(paths: LocalPaths, profiles: ProfileRecord[]) {
  if (!profiles.every((profile) => validProfile(profile, paths.home)))
    throw new Error('Invalid managed shell profile path')
  const removed: string[] = []
  for (const profile of profiles) {
    ownedPath(profile.path, paths.home)
    const saved = read(profile.path)
    if (saved.created) continue
    if (!saved.text.includes(BLOCK)) continue
    const text = saved.text.replace(BLOCK, '')
    if (profile.created && !text) unlinkSync(profile.path)
    else createPrivateFile(profile.path, text, true, saved.mode)
    removed.push(profile.path)
  }
  return removed
}
