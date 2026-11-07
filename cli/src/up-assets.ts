// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Exact-release download and SHA256 verification before any executable is installed. */
import { createHash } from 'node:crypto'
import {
  chmodSync,
  copyFileSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  renameSync,
  writeFileSync,
} from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { PRYX_VERSION } from '../../control/src/generated/release.ts'
import { BOOTSTRAP_SIGNATURE_ASSET } from '../../control/src/join-bootstrap.ts'
export const FLEET_HANDOUT_DIR = '/opt/pryx/fleet'
export const FLEET_PRYXD_BIN = `${FLEET_HANDOUT_DIR}/pryxd`
export const RELEASE_FILES = [
  'pryxd',
  'pryx-agent',
  'pryxd.sig',
  BOOTSTRAP_SIGNATURE_ASSET,
  'pryx-control',
  'pryx-edge',
  'pryx-control-assets.tar.gz',
] as const
export async function releaseFiles(source?: string) {
  const dir = source ?? mkdtempSync(join(tmpdir(), 'pryx-release-'))
  if (!source) {
    mkdirSync(dir, { recursive: true })
    const base =
      process.env.PRYX_RELEASE_BASE_URL ??
      `https://github.com/cubepals/pryx/releases/download/v${PRYX_VERSION}`
    const origin = new URL(base)
    if (origin.protocol !== 'https:' && !(origin.protocol === 'http:' && origin.hostname === '127.0.0.1'))
      throw new Error('Release downloads require HTTPS or loopback HTTP artifacts')
    for (const file of ['SHA256SUMS', ...RELEASE_FILES]) {
      const response = await fetch(`${base}/${file}`)
      if (!response.ok) throw new Error(`Release asset ${file} unavailable`)
      const destination = new URL(response.url)
      if (
        destination.protocol !== 'https:' &&
        !(
          origin.protocol === 'http:' &&
          destination.protocol === 'http:' &&
          destination.hostname === '127.0.0.1'
        )
      )
        throw new Error('Release redirect left its secure transport')
      writeFileSync(join(dir, file), new Uint8Array(await response.arrayBuffer()))
    }
  }
  const sums = new Map(
    readFileSync(join(dir, 'SHA256SUMS'), 'utf8')
      .trim()
      .split('\n')
      .map((line) => {
        const match = /^([a-f0-9]{64}) {2}(.+)$/.exec(line)
        if (!match) throw new Error('Invalid release checksum manifest')
        return [match[2] as string, match[1] as string]
      }),
  )
  for (const file of RELEASE_FILES) {
    const digest = createHash('sha256')
      .update(readFileSync(join(dir, file)))
      .digest('hex')
    if (sums.get(file) !== digest) throw new Error(`Release asset ${file} failed SHA256 verification`)
  }
  return dir
}
export function installBinary(source: string, target: string) {
  const wanted = readFileSync(source)
  if (existsSync(target) && readFileSync(target).equals(wanted)) return false
  mkdirSync(join(target, '..'), { recursive: true })
  copyFileSync(source, `${target}.next`)
  chmodSync(`${target}.next`, 0o755)
  renameSync(`${target}.next`, target)
  return true
}
