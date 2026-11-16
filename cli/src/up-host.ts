// SPDX-License-Identifier: FSL-1.1-ALv2
/** Host Docker and firewall prerequisites for installation. Merges the existing daemon policy,
 * reloads an active engine, and never removes existing containers or unrelated settings. */
import { existsSync, mkdirSync, readFileSync, renameSync, rmSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import daemonJson from '../../deploy/daemon.json'
import gvisor from '../../deploy/gvisor.json'
import { writeChanged } from './up-files.ts'
export type InstallCommand = (args: string[], quiet?: boolean, redactions?: string[]) => Promise<void>
export function hostInstallProblem(docker: boolean, nft: boolean, distribution: string): string | null {
  if ((docker && nft) || ['debian', 'ubuntu'].includes(distribution)) return null
  const missing = [!docker && 'Docker', !nft && 'nftables'].filter(Boolean).join(' and ')
  return `Missing ${missing}; automatic package installation is unavailable on ${distribution}. Install ${missing} with this distribution's package manager, then run pryx up`
}
export function checkHostRuntime() {
  const release = readFileSync('/etc/os-release', 'utf8')
  const distribution = /^ID=(.*)$/m.exec(release)?.[1]?.replaceAll('"', '') ?? 'unknown Linux distribution'
  const problem = hostInstallProblem(Boolean(Bun.which('docker')), Boolean(Bun.which('nft')), distribution)
  if (problem) throw new Error(problem)
}
const DOCKER_INSTALL = `set -eu
. /etc/os-release
case "$ID" in ubuntu|debian) ;; *) exit 1;; esac
apt-get update -q
apt-get install -yq ca-certificates curl
install -d -m 0755 /etc/apt/keyrings
curl -fsSL "https://download.docker.com/linux/$ID/gpg" -o /etc/apt/keyrings/docker.asc
chmod a+r /etc/apt/keyrings/docker.asc
printf 'deb [arch=%s signed-by=/etc/apt/keyrings/docker.asc] https://download.docker.com/linux/%s %s stable\n' "$(dpkg --print-architecture)" "$ID" "$VERSION_CODENAME" > /etc/apt/sources.list.d/docker.list
apt-get update -q
apt-get install -yq docker-ce docker-ce-cli containerd.io
`
/** Where the server preset keeps gVisor's runsc and the sidecar programs its release ships. */
export const GVISOR_PREFIX = '/opt/pryx/gvisor'

/** The engine configuration `runsc install` writes: the runtime under its own name. */
export function gvisorRuntimes(
  existing: { runtimes?: Record<string, unknown> },
  prefix = GVISOR_PREFIX,
): Record<string, unknown> {
  return { ...existing.runtimes, runsc: { path: join(prefix, 'runsc') } }
}

/** gVisor's runsc at deploy/gvisor.json's release, checked against its SHA-512, unpacked as is. */
export async function installGvisor(run: InstallCommand, prefix = GVISOR_PREFIX, url = gvisor.url) {
  const runsc = join(prefix, 'runsc')
  const installed = existsSync(runsc) ? Bun.spawnSync([runsc, '--version'], { stderr: 'ignore' }) : undefined
  if (installed?.exitCode === 0 && installed.stdout.toString().includes(`runsc version ${gvisor.version}\n`))
    return
  const response = await fetch(url)
  if (!response.ok) throw new Error(`the gVisor release answered ${response.status}`)
  const bytes = new Uint8Array(await response.arrayBuffer())
  if (new Bun.CryptoHasher('sha512').update(bytes).digest('hex') !== gvisor.sha512)
    throw new Error('the gVisor release does not match its pinned SHA-512')
  const staged = `${prefix}.new`
  rmSync(staged, { recursive: true, force: true })
  mkdirSync(staged, { recursive: true, mode: 0o755 })
  writeFileSync(join(staged, 'gvisor.tar'), Bun.zstdDecompressSync(bytes))
  await run(['tar', '-x', '--no-same-owner', '-f', join(staged, 'gvisor.tar'), '-C', staged])
  rmSync(join(staged, 'gvisor.tar'))
  rmSync(prefix, { recursive: true, force: true })
  renameSync(staged, prefix)
}

export async function configureHostRuntime(run: InstallCommand) {
  checkHostRuntime()
  if (!Bun.which('docker')) await run(['sh', '-c', DOCKER_INSTALL])
  if (!Bun.which('nft')) {
    await run(['apt-get', 'update', '-q'])
    await run(['apt-get', 'install', '-yq', 'nftables'])
  }
  await run(['systemctl', 'enable', '--now', 'docker'])
  const existing = existsSync('/etc/docker/daemon.json')
    ? JSON.parse(readFileSync('/etc/docker/daemon.json', 'utf8'))
    : {}
  // The gvisor tier is the engine's runsc; where it can't be installed, that tier alone is off.
  let runtimes: { runtimes?: Record<string, unknown> } = {}
  if (process.arch === 'x64')
    try {
      await installGvisor(run)
      runtimes = { runtimes: gvisorRuntimes(existing) }
    } catch (error) {
      console.error(`gvisor tier off: ${error instanceof Error ? error.message : error}`)
    }
  if (
    writeChanged(
      '/etc/docker/daemon.json',
      JSON.stringify({ ...existing, ...daemonJson, ...runtimes }, null, 2) + '\n',
      0o644,
    )
  ) {
    const running = Bun.spawnSync(['docker', 'ps', '-q']).stdout.toString().trim()
    if (running) await run(['systemctl', 'reload', 'docker'])
    else await run(['systemctl', 'restart', 'docker'])
  }
}
