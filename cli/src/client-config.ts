// SPDX-License-Identifier: FSL-1.1-ALv2
/** User-owned CLI credentials make the literal quickstart work after sudo up. Only the public API key is copied; operator, CA and store credentials stay root-only. */
import {
  chmodSync,
  closeSync,
  constants,
  existsSync,
  fstatSync,
  lstatSync,
  mkdirSync,
  openSync,
  readFileSync,
  renameSync,
  unlinkSync,
  writeFileSync,
} from 'node:fs'
import { homedir } from 'node:os'
import { dirname, join } from 'node:path'

export type ClientConfig = { url: string; key: string }
function parseConfig(value: unknown): ClientConfig {
  if (
    !value ||
    typeof value !== 'object' ||
    !('url' in value) ||
    typeof value.url !== 'string' ||
    !('key' in value) ||
    typeof value.key !== 'string' ||
    !value.key ||
    Object.keys(value).length !== 2
  )
    throw new Error('Invalid CLI credentials')
  const url = new URL(value.url)
  if (!['http:', 'https:'].includes(url.protocol) || url.username || url.password)
    throw new Error('Invalid CLI API URL')
  return { url: value.url, key: value.key }
}
export const clientConfigPath = () => process.env.PRYX_CONFIG ?? join(homedir(), '.config/pryx/client.json')
export function readClientConfig(path = clientConfigPath()): ClientConfig | undefined {
  if (!existsSync(path)) return undefined
  const fd = openSync(path, constants.O_RDONLY | constants.O_NOFOLLOW)
  try {
    const stat = fstatSync(fd)
    if (!stat.isFile() || stat.uid !== process.getuid?.() || (stat.mode & 0o077) !== 0)
      throw new Error('CLI credentials must be an owner-only regular file')
    return parseConfig(JSON.parse(readFileSync(fd, 'utf8')))
  } finally {
    closeSync(fd)
  }
}
export function writeClientConfig(config: ClientConfig, path = clientConfigPath()) {
  const directory = dirname(path)
  mkdirSync(directory, { recursive: true, mode: 0o700 })
  const stat = lstatSync(directory)
  if (!stat.isDirectory() || stat.uid !== process.getuid?.())
    throw new Error('CLI credential directory must belong to its user')
  chmodSync(directory, 0o700)
  const temporary = join(directory, `.client-${crypto.randomUUID()}`)
  const fd = openSync(
    temporary,
    constants.O_CREAT | constants.O_EXCL | constants.O_WRONLY | constants.O_NOFOLLOW,
    0o600,
  )
  try {
    writeFileSync(fd, `${JSON.stringify(parseConfig(config))}\n`)
  } finally {
    closeSync(fd)
  }
  try {
    renameSync(temporary, path)
  } finally {
    if (existsSync(temporary)) unlinkSync(temporary)
  }
  return path
}
export function callerHomeAndDropPrivileges(): string {
  if (process.getuid?.() !== 0 || !process.env.SUDO_UID || process.env.SUDO_UID === '0') return homedir()
  if (!/^\d+$/.test(process.env.SUDO_UID)) throw new Error('Invalid sudo caller')
  const record = Bun.spawnSync(['getent', 'passwd', process.env.SUDO_UID])
  const fields = record.stdout.toString().trim().split(':')
  const uid = Number(fields[2]),
    gid = Number(fields[3]),
    home = fields[5]
  if (
    record.exitCode !== 0 ||
    uid !== Number(process.env.SUDO_UID) ||
    !Number.isSafeInteger(gid) ||
    !home?.startsWith('/') ||
    !lstatSync(home).isDirectory()
  )
    throw new Error('Cannot resolve the sudo caller home')
  if (!process.setgroups || !process.setgid || !process.setuid)
    throw new Error('Cannot drop installer privileges on this runtime')
  process.setgroups([])
  process.setgid(gid)
  process.setuid(uid)
  if (process.getuid?.() !== uid || process.getgid?.() !== gid)
    throw new Error('Installer privilege drop failed')
  return home
}
