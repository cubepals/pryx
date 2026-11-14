// SPDX-License-Identifier: FSL-1.1-ALv2
/** Bounded release downloads and digest checks. An authenticated archive is still staged
 * privately; this module never activates a bundle or opens a public listener. */
import { createHash } from 'node:crypto'
import { createWriteStream, existsSync, readFileSync } from 'node:fs'
import { join } from 'node:path'
import { pipeline } from 'node:stream/promises'
import { HOST_BUNDLE } from '../../control/src/bundle-release.ts'
import { MINUTE_MS } from '../../control/src/durations.ts'
import { PRYX_VERSION } from '../../control/src/generated/release.ts'

// A staging budget, not an expected artifact size. Signed release contents determine extraction.
const MAX_BUNDLE_BYTES = 1024 ** 3
const MAX_MANIFEST_BYTES = 64 * 1024
export const releaseBase = () =>
  process.env.PRYX_RELEASE_BASE_URL ?? `https://github.com/cubepals/pryx/releases/download/v${PRYX_VERSION}`
export async function selectedRelease(upgrading: boolean, source?: string) {
  if (source || !upgrading || process.env.PRYX_RELEASE_BASE_URL) {
    const file = source && join(source, 'VERSION')
    const version = file && existsSync(file) ? readFileSync(file, 'utf8').trim() : PRYX_VERSION
    if (!/^\d+\.\d+\.\d+$/.test(version)) throw new Error('Invalid release version')
    return { version, base: releaseBase() }
  }
  const response = await fetch('https://api.github.com/repos/cubepals/pryx/releases/latest', {
    headers: { accept: 'application/vnd.github+json' },
    signal: AbortSignal.timeout(MINUTE_MS),
  })
  if (!response.ok) throw new Error('The latest release could not be resolved; current state was kept')
  const release = (await response.json()) as { tag_name?: string }
  const version = /^v(\d+\.\d+\.\d+)$/.exec(release.tag_name ?? '')?.[1]
  if (!version) throw new Error('The latest release has an unsupported version; current state was kept')
  return { version, base: `https://github.com/cubepals/pryx/releases/download/v${version}` }
}
function secure(url: URL, loopback: boolean) {
  return (
    !url.username &&
    !url.password &&
    (url.protocol === 'https:' || (loopback && url.protocol === 'http:' && url.hostname === '127.0.0.1'))
  )
}
async function download(base: string, name: string, target: string, maximum: number) {
  const origin = new URL(base)
  const loopback = origin.protocol === 'http:' && origin.hostname === '127.0.0.1'
  if (!secure(origin, loopback)) throw new Error('Release downloads require HTTPS or loopback HTTP artifacts')
  const response = await fetch(`${base}/${name}`, { signal: AbortSignal.timeout(5 * MINUTE_MS) })
  if (!response.ok || !response.body) throw new Error(`Release asset ${name} unavailable`)
  if (!secure(new URL(response.url), loopback)) throw new Error('Release redirect left its secure transport')
  await pipeline(
    response.body,
    async function* (source) {
      let bytes = 0
      for await (const chunk of source) {
        bytes += chunk.byteLength
        if (bytes > maximum) throw new Error(`Release asset ${name} exceeds the staging budget`)
        yield chunk
      }
    },
    createWriteStream(target, { flags: 'wx', mode: 0o600 }),
  )
}
export async function hashFile(path: string) {
  const hash = createHash('sha256')
  for await (const chunk of Bun.file(path).stream()) hash.update(chunk)
  return hash.digest('hex')
}
export async function bundleFiles(directory: string, source?: string, base = releaseBase()) {
  if (!source) {
    for (const [name, limit] of [
      ['SHA256SUMS', MAX_MANIFEST_BYTES],
      [`${HOST_BUNDLE}.sig`, MAX_MANIFEST_BYTES],
      [HOST_BUNDLE, MAX_BUNDLE_BYTES],
    ] as const)
      await download(base, name, join(directory, name), limit)
  }
  const location = source ?? directory
  const expected = readFileSync(join(location, 'SHA256SUMS'), 'utf8')
    .split('\n')
    .flatMap((line) => {
      const entry = /^([a-f0-9]{64}) [ *](.+)$/.exec(line)
      return entry?.[2] === HOST_BUNDLE ? [entry[1] as string] : []
    })
  const archive = join(location, HOST_BUNDLE)
  const digest = await hashFile(archive)
  if (expected.length !== 1 || digest !== expected[0])
    throw new Error('Host bundle SHA256 verification failed')
  return { archive, digest, signature: readFileSync(join(location, `${HOST_BUNDLE}.sig`)) }
}
