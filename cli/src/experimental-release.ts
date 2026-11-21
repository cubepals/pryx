// SPDX-License-Identifier: FSL-1.1-ALv2
/** Authenticate the optional per-user lab archive before handing it a request. The default
 * install never fetches it. Experiment implementation and native binaries live in the archive. */
import { copyFileSync, existsSync, lstatSync, mkdtempSync, readFileSync, readlinkSync, rmSync } from 'node:fs'
import { homedir, tmpdir } from 'node:os'
import { join } from 'node:path'
import { PRYX_VERSION } from '../../control/src/generated/release.ts'
import { download, hashFile, releaseBase } from './bundle-download.ts'
import { trustedSignature } from './bundle-trust.ts'
import { ownedPath } from './up-paths.ts'

export const requestsLab = (args: string[]) =>
  args.includes('--experimental') || process.env.PRYX_EXPERIMENTAL === '1'
const root = () => join(homedir(), '.pryx-experimental')
export const labInstalled = () => existsSync(join(root(), 'active'))

function labStatement(digest: string) {
  if (!['x64', 'arm64'].includes(process.arch) || process.platform !== 'linux')
    throw new Error('The lab requires x86_64 or aarch64 Linux')
  return Buffer.from(`pryx experimental archive\n${PRYX_VERSION}\n${process.arch}\n${digest}\n`)
}

type Files = { archive: string; signature: string }
async function staged<T>(source: (files: Files) => Promise<void>, action: (files: Files) => Promise<T>) {
  const temp = mkdtempSync(join(tmpdir(), 'pryx-experimental-'))
  try {
    const files = { archive: join(temp, 'lab.pyz'), signature: join(temp, 'signature') }
    await source(files)
    const signature = readFileSync(files.signature)
    if (!trustedSignature(labStatement(await hashFile(files.archive)), signature))
      throw new Error('Experimental archive signature does not match this release and architecture')
    return await action(files)
  } finally {
    rmSync(temp, { force: true, recursive: true })
  }
}

const maxSize = 32 * 1024 * 1024
async function fetchLab(files: Files) {
  const name = `pryx-lab-linux-${process.arch}.pyz`
  await download(releaseBase(), `${name}.sig`, files.signature, 64)
  await download(releaseBase(), name, files.archive, maxSize)
}
function invoke(files: Files, args: string[]) {
  const result = Bun.spawnSync(['python3', '-I', files.archive, ...args], {
    stdout: 'pipe',
    stderr: 'inherit',
  })
  if (result.exitCode !== 0) throw new Error('Experimental lab command failed')
  return result.stdout.toString()
}

export async function experimentalUp<T>(
  args: string[],
  base: (args: string[], onReady?: () => void) => Promise<T>,
) {
  const lab: string[] = [],
    stock: string[] = []
  for (let index = 0; index < args.length; index++) {
    const arg = args[index] as string
    if (arg === '--feature') lab.push(arg, args[++index] ?? '')
    else if (arg !== '--experimental') stock.push(arg)
  }
  if (!requestsLab(args)) throw new Error('--feature requires --experimental or PRYX_EXPERIMENTAL=1')
  // Preflight stays offline. This release exposes one public feature name; its implementation
  // and execution remain entirely in the signed archive.
  if (lab.length && (lab.length !== 2 || lab[1] !== 'self-check'))
    throw new Error('Expected --feature self-check')
  if (stock.includes('--check') || stock.includes('--preflight')) return await base(stock)
  return await staged(fetchLab, async (files) => {
    invoke(files, ['validate', ...lab])
    let activated = false
    const activate = () => {
      if (activated) return
      invoke(files, ['activate', files.signature, ...lab])
      activated = true
      // Foreground presets stay in base until shutdown. Discard the verified staging files now.
      rmSync(files.archive)
      rmSync(files.signature)
    }
    const result = await base(stock, activate)
    activate()
    return result
  })
}

export async function installedLab(command: 'capabilities' | 'run', args: string[] = []) {
  if (!labInstalled()) {
    if (command === 'run') throw new Error('Install the experimental lab and select the feature first')
    return { installed: false, reason: 'experimental lab has not been installed' }
  }
  const directory = ownedPath(root(), homedir())
  const target = readlinkSync(join(directory, 'active'))
  if (!/^build-[a-f0-9]{64}$/.test(target)) throw new Error('Invalid experimental installation')
  const member = (name: string, size: number) => {
    const path = ownedPath(join(directory, target, name), homedir())
    const info = lstatSync(path)
    if (!info.isFile() || info.size > size) throw new Error('Invalid experimental installation member')
    return path
  }
  const selection = readFileSync(member('selection', 256), 'utf8')
  return await staged(
    async (files) => {
      copyFileSync(member('lab.pyz', maxSize), files.archive)
      copyFileSync(member('signature', 64), files.signature)
    },
    async (files) => {
      const output = invoke(files, [command, selection, ...args])
      if (command === 'run') {
        process.stdout.write(output)
        return
      }
      return JSON.parse(output)
    },
  )
}
