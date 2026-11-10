// SPDX-License-Identifier: FSL-1.1-ALv2
/** Privately stage a signature-verified host release, then move it into its digest directory.
 * State, credentials and the active installation record are not changed here. */
import { chmodSync, cpSync, mkdirSync, mkdtempSync, readFileSync, renameSync, rmSync } from 'node:fs'
import { join } from 'node:path'
import { PRYX_VERSION } from '../../control/src/generated/release.ts'
import { bundleFiles } from './bundle-download.ts'
import { verifyBundleSignature } from './bundle-trust.ts'
import { type LocalPaths, ownedPath } from './up-paths.ts'

export type BundleMetadata = {
  version: string
  architecture: string
  postgres: { major: string; version: string }
  serviceUid: number
  serviceGid: number
}
export async function stageBundle(
  paths: LocalPaths,
  options: { source?: string; trustedKeys?: string; version?: string; base?: string } = {},
) {
  if (process.getuid?.() !== 0) ownedPath(paths.cache, paths.home)
  mkdirSync(paths.cache, { recursive: true, mode: 0o700 })
  const temporary = mkdtempSync(join(paths.cache, 'bundle-'))
  try {
    const files = await bundleFiles(temporary, options.source, options.base)
    const version = options.version ?? PRYX_VERSION
    verifyBundleSignature(version, files.digest, files.signature, options.trustedKeys)
    const unpacked = join(temporary, 'unpacked')
    mkdirSync(unpacked, { mode: 0o700 })
    const listing = Bun.spawnSync(['tar', '-tzf', files.archive], { stdout: 'pipe', stderr: 'pipe' })
    if (
      listing.exitCode ||
      listing.stdout
        .toString()
        .split('\n')
        .some((name) => name.startsWith('/') || name.split('/').includes('..'))
    )
      throw new Error('Host bundle contains an unsafe archive path')
    const types = Bun.spawnSync(['tar', '-tvzf', files.archive], { stdout: 'pipe', stderr: 'pipe' })
    if (
      types.exitCode ||
      types.stdout
        .toString()
        .trimEnd()
        .split('\n')
        .some((line) => !/^[d-][rwx-]{9}\s/.test(line))
    )
      throw new Error(
        'Host bundle permits only ordinary files and directories; links and special modes are refused',
      )
    const extract = Bun.spawn(['tar', '--no-same-owner', '-xzf', files.archive, '-C', unpacked], {
      stdout: 'ignore',
      stderr: 'pipe',
    })
    const [, status] = await Promise.all([new Response(extract.stderr).text(), extract.exited])
    if (status) throw new Error('Host bundle extraction failed')
    const metadata = JSON.parse(readFileSync(join(unpacked, 'bundle.json'), 'utf8')) as BundleMetadata
    if (
      metadata.version !== version ||
      metadata.architecture !== 'x86_64' ||
      !Number.isSafeInteger(metadata.serviceUid) ||
      metadata.serviceUid <= 0 ||
      !Number.isSafeInteger(metadata.serviceGid) ||
      metadata.serviceGid <= 0
    )
      throw new Error('Host bundle metadata does not match this release')
    return { temporary, unpacked, digest: files.digest, metadata }
  } catch (error) {
    rmSync(temporary, { recursive: true, force: true })
    throw error
  }
}
export function retainBundle(paths: LocalPaths, staged: Awaited<ReturnType<typeof stageBundle>>) {
  const directory = join(paths.data, 'releases')
  if (process.getuid?.() !== 0) ownedPath(directory, paths.home)
  const mode = process.getuid?.() === 0 ? 0o755 : 0o700
  mkdirSync(directory, { recursive: true, mode })
  // Every staging operation has a new directory even when a preview reuses the release version.
  const bundle = join(directory, `${staged.digest}-${crypto.randomUUID()}`)
  try {
    renameSync(staged.unpacked, bundle)
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== 'EXDEV') throw error
    // The cache and data XDG directories may live on different filesystems.
    const next = `${bundle}.next`
    try {
      cpSync(staged.unpacked, next, { recursive: true, errorOnExist: true, force: false })
      renameSync(next, bundle)
    } finally {
      rmSync(next, { recursive: true, force: true })
    }
  }
  chmodSync(bundle, mode)
  rmSync(staged.temporary, { recursive: true, force: true })
  return bundle
}
