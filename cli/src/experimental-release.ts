// SPDX-License-Identifier: FSL-1.1-ALv2
/** Verify an opt-in release before executing its controller. Only this trust boundary ships
 * in the stock CLI. The authenticated controller owns all feature and activation policy. */
import { copyFileSync, lstatSync, mkdtempSync, readFileSync, readlinkSync, rmSync } from 'node:fs'
import { homedir, tmpdir } from 'node:os'
import { join } from 'node:path'
import { PRYX_VERSION } from '../../control/src/generated/release.ts'
import { download, hashFile, releaseBase } from './bundle-download.ts'
import { trustedSignature } from './bundle-trust.ts'
import { ownedPath } from './up-paths.ts'

export function requestsLab(args: string[]) {
  if (process.env.PRYX_EXPERIMENTAL === '1' || args.includes('--experimental')) return true
  if (args.includes('--feature')) throw new Error('--feature requires --experimental or PRYX_EXPERIMENTAL=1')
  return false
}
const root = () => join(homedir(), '.pryx-experimental')
export function labInstalled() {
  try {
    return Boolean(lstatSync(join(root(), 'active'), { throwIfNoEntry: false }))
  } catch {
    // An unreadable activation must still produce a diagnostic beside the ordinary host probe.
    return true
  }
}

export async function handoff(fresh: boolean, command: string, args: string[], stock?: unknown) {
  if (fresh && !requestsLab(args))
    throw new Error('Installing the lab requires --experimental or PRYX_EXPERIMENTAL=1')
  if (process.platform !== 'linux' || !['x64', 'arm64'].includes(process.arch))
    throw new Error('The lab requires x86_64 or aarch64 Linux')
  const temp = mkdtempSync(join(tmpdir(), 'pryx-experimental-'))
  try {
    let generation: string | undefined
    if (!fresh) {
      if (!labInstalled()) throw new Error('Experimental lab has not been installed')
      const parent = ownedPath(root(), homedir())
      const target = readlinkSync(join(parent, 'active'))
      if (!/^[a-z0-9-]+$/.test(target)) throw new Error('Invalid experimental installation directory')
      generation = ownedPath(join(parent, target), homedir())
    }
    const name = `pryx-lab-linux-${process.arch}.pyz`
    for (const [local, asset, limit] of [
      ['signature', `${name}.sig`, 64],
      ['lab.pyz', name, 32 * 1024 ** 2],
    ] as const) {
      const destination = join(temp, local)
      if (!generation) await download(releaseBase(), asset, destination, limit)
      else {
        const file = ownedPath(join(generation, local), homedir())
        const stat = lstatSync(file)
        if (!stat.isFile() || stat.size > limit) throw new Error('Invalid experimental release file')
        copyFileSync(file, destination)
      }
    }
    const archive = join(temp, 'lab.pyz'),
      signature = join(temp, 'signature')
    const statement = Buffer.from(
      `pryx experimental archive\n${PRYX_VERSION}\n${process.arch}\n${await hashFile(archive)}\n`,
    )
    if (!trustedSignature(statement, readFileSync(signature)))
      throw new Error('Experimental archive signature does not match this release and architecture')
    const code = Bun.spawnSync(['python3', '-I', archive, 'controller'], { stdout: 'pipe', stderr: 'pipe' })
    if (code.exitCode !== 0)
      throw new Error(code.stderr.toString().trim().slice(-512) || 'Experimental controller unavailable')
    const controller = await import(`data:text/javascript;base64,${code.stdout.toString('base64')}`)
    return await controller.default({ archive, signature, generation }, command, args, stock)
  } finally {
    rmSync(temp, { recursive: true, force: true })
  }
}
