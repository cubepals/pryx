// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Atomic installer-owned files and unit environments; credentials are never included in diagnostics. */
import { randomUUID } from 'node:crypto'
import { chmodSync, existsSync, mkdirSync, readFileSync, renameSync, statSync, writeFileSync } from 'node:fs'
import { dirname } from 'node:path'
export function writeChanged(path: string, contents: string | Uint8Array, mode: number) {
  mkdirSync(dirname(path), { recursive: true })
  if (existsSync(path) && readFileSync(path).equals(Buffer.from(contents))) {
    if ((statSync(path).mode & 0o777) !== mode) chmodSync(path, mode)
    return false
  }
  const temporary = `${path}.next-${randomUUID()}`
  writeFileSync(temporary, contents, { mode, flag: 'wx' })
  chmodSync(temporary, mode)
  renameSync(temporary, path)
  return true
}
export function readEnvironment(path: string): Record<string, string> {
  if (!existsSync(path)) return {}
  const result: Record<string, string> = {}
  for (const line of readFileSync(path, 'utf8').split('\n')) {
    if (!line || line.startsWith('#')) continue
    const at = line.indexOf('=')
    if (at < 1) throw new Error('Invalid managed environment file')
    result[line.slice(0, at)] = JSON.parse(line.slice(at + 1)) as string
  }
  return result
}
export const writeEnvironment = (path: string, env: Record<string, string>) =>
  writeChanged(
    path,
    Object.entries(env)
      .map(([k, v]) => `${k}=${JSON.stringify(v)}`)
      .join('\n') + '\n',
    0o600,
  )
