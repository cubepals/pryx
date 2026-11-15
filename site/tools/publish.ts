// SPDX-License-Identifier: FSL-1.1-ALv2
/** Release-only static export to an explicitly selected existing webroot. This is filesystem
 * publication glue, not a host provisioner, web server or provider account integration. */
import {
  chmodSync,
  cpSync,
  lstatSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  renameSync,
  rmSync,
  writeFileSync,
} from 'node:fs'
import { basename, dirname, join, parse, resolve } from 'node:path'
import { siteAddress } from '../../control/src/site-address.ts'
import { fileDigests, files, sourceGit, verifyArtifact } from './artifact.ts'

export type SiteManifest = {
  version: string
  base: string
  address?: string
  commit: string
  files: Record<string, string>
}
const pointerMarker = '<!-- pryx documentation version pointer -->'
const entryExists = (path: string) => readdirSync(dirname(path)).includes(basename(path))
export function assertDirectory(path: string) {
  const absolute = resolve(path)
  let at = parse(absolute).root
  for (const part of absolute.slice(at.length).split('/').filter(Boolean)) {
    at = join(at, part)
    const stat = lstatSync(at)
    if (stat.isSymbolicLink() || !stat.isDirectory())
      throw new Error('Publication path must contain only existing directories')
  }
  return absolute
}
export function assertRelease(repository: string, version: string, commit: string) {
  const git = (...args: string[]) => sourceGit(repository, ...args)
  if (resolve(git('rev-parse', '--show-toplevel')) !== resolve(repository))
    throw new Error('Release checkout path differs')
  if (!/^[\da-zA-Z.+-]+$/.test(version)) throw new Error('Invalid release version')
  if (git('status', '--porcelain')) throw new Error('Publish only a clean release checkout')
  let released: string
  try {
    released = git('rev-parse', `refs/tags/v${version}^{commit}`)
  } catch {
    throw new Error('This site version has no local release tag')
  }
  if (git('rev-parse', 'HEAD') !== commit || released !== commit)
    throw new Error('Site source must be the exact release tag')
}
export function publishSite(options: {
  repository: string
  artifact: string
  webroot: string
  url: string
  manifest: SiteManifest
}) {
  const { manifest } = options
  const configured = siteAddress(options.url).configured
  if (
    !configured ||
    configured !== manifest.address ||
    `${new URL(configured).pathname}v${manifest.version}/` !== manifest.base
  )
    throw new Error('Build and publication addresses differ')
  assertRelease(options.repository, manifest.version, manifest.commit)
  const webroot = assertDirectory(options.webroot)
  const artifact = assertDirectory(options.artifact)
  verifyArtifact(artifact, manifest.files)
  if (!manifest.files['index.html']) throw new Error('Static artifact has no index')
  const current = join(webroot, 'index.html')
  if (
    entryExists(current) &&
    (!lstatSync(current).isFile() ||
      lstatSync(current).isSymbolicLink() ||
      !readFileSync(current, 'utf8').includes(pointerMarker))
  )
    throw new Error('Publication refuses an unowned or linked root index')
  const destination = join(webroot, `v${manifest.version}`)
  if (entryExists(destination)) {
    assertDirectory(destination)
    verifyArtifact(destination, manifest.files)
  } else {
    const staging = mkdtempSync(join(webroot, '.pryx-site-'))
    try {
      cpSync(artifact, staging, { recursive: true, dereference: false })
      verifyArtifact(staging, manifest.files)
      for (const file of files(staging)) chmodSync(join(staging, file), 0o644)
      const directories = new Set(files(staging).map((file) => dirname(file)))
      for (const directory of directories) {
        let at = resolve(staging, directory)
        while (at.startsWith(`${staging}/`)) {
          chmodSync(at, 0o755)
          at = dirname(at)
        }
      }
      chmodSync(staging, 0o755)
      try {
        renameSync(staging, destination)
      } catch (error) {
        if (!entryExists(destination)) throw error
        assertDirectory(destination)
        verifyArtifact(destination, manifest.files)
      }
    } finally {
      rmSync(staging, { recursive: true, force: true })
    }
  }
  const pointer = mkdtempSync(join(webroot, '.pryx-pointer-'))
  try {
    const path = join(pointer, 'index.html')
    const target = `v${manifest.version}/`
    writeFileSync(
      path,
      `<!doctype html>${pointerMarker}<meta charset="utf-8"><meta http-equiv="refresh" content="0;url=${target}"><title>pryx documentation</title><a href="${target}">Open pryx documentation ${manifest.version}</a>`,
      { mode: 0o644 },
    )
    chmodSync(path, 0o644)
    renameSync(path, current)
  } finally {
    rmSync(pointer, { recursive: true, force: true })
  }
  return {
    version: manifest.version,
    url: `${configured}v${manifest.version}/`,
    files: Object.keys(fileDigests(destination)).length,
  }
}
