// SPDX-License-Identifier: FSL-1.1-ALv2
/** Host Docker and firewall prerequisites for installation. Merges the existing daemon policy,
 * reloads an active engine, and never removes existing containers or unrelated settings. */
import {
  closeSync,
  constants,
  existsSync,
  mkdirSync,
  mkdtempSync,
  openSync,
  readFileSync,
  readlinkSync,
  renameSync,
  rmSync,
  writeFileSync,
} from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { MINUTE_MS, SECOND_MS } from '../../control/src/durations.ts'
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
  prefix = join(GVISOR_PREFIX, gvisor.version),
): Record<string, unknown> {
  return { ...existing.runtimes, runsc: { path: join(prefix, 'runsc') } }
}

// Moby daemon/config/config_linux.go's unexported lookupBinPath searches these FHS
// directories before PATH: https://github.com/moby/moby/blob/v28.0.4/daemon/config/config_linux.go.
const DOCKER_LIBEXEC_DIRECTORIES = [
  '/usr/local/libexec/docker',
  '/usr/libexec/docker',
  '/usr/local/lib/docker',
  '/usr/lib/docker',
  '/usr/local/libexec',
  '/usr/libexec',
]
// Moby cmd/dockerd/config_unix.go's flag. Only its presence is returned, never other arguments.
const DOCKER_PROXY_ARGUMENT = '--userland-proxy-path'
// Docker25 added this platform validation; older reloads accept the partial proxy config.
// https://github.com/moby/moby/blob/v25.0.0/daemon/config/config_linux.go
const PROXY_RELOAD_VALIDATION_MAJOR = 25

/** Anchor proc lookups to the identified daemon task, so PID reuse cannot select another
 * command line. Argument values stay private; only the proxy flag's presence leaves here. */
export function dockerProxyArgument(pid: number): boolean {
  if (!Number.isSafeInteger(pid) || pid <= 1) throw new Error('Docker service has no active daemon PID')
  const directory = openSync(
    `/proc/${pid}`,
    constants.O_RDONLY | constants.O_DIRECTORY | constants.O_NOFOLLOW,
  )
  try {
    const proc = `/proc/self/fd/${directory}`
    if (
      !readlinkSync(`${proc}/exe`)
        .replace(/ \(deleted\)$/, '')
        .endsWith('/dockerd')
    )
      throw new Error('Docker service does not identify its daemon executable')
    return readFileSync(`${proc}/cmdline`)
      .toString()
      .split('\0')
      .some(
        (argument) => argument === DOCKER_PROXY_ARGUMENT || argument.startsWith(`${DOCKER_PROXY_ARGUMENT}=`),
      )
  } finally {
    closeSync(directory)
  }
}

function serviceProxyArgument(): boolean {
  const child = Bun.spawnSync(['systemctl', 'show', 'docker', '--property=MainPID', '--value'], {
    stdout: 'pipe',
    stderr: 'ignore',
    timeout: SECOND_MS,
  })
  if (child.exitCode !== 0) throw new Error('Cannot identify Docker service for a safe live runtime reload')
  return dockerProxyArgument(Number(child.stdout.toString().trim()))
}

function dockerMajor(): number {
  const child = Bun.spawnSync(['docker', 'version', '--format', '{{.Server.Version}}'], {
    stdout: 'pipe',
    stderr: 'ignore',
    timeout: SECOND_MS,
  })
  const major = /^(\d+)\./.exec(child.stdout.toString().trim())?.[1]
  if (child.exitCode !== 0 || !major) throw new Error('Cannot identify Docker proxy reload requirements')
  return Number(major)
}

/** Docker validates a reload's file before merging defaults. An enabled proxy therefore
 * needs its executable in that file; preserve disabled proxies and explicit paths as given. */
export function dockerReloadConfiguration<T extends Record<string, unknown>>(
  configuration: T,
  find: (path: string) => string | null = Bun.which,
  commandLineProxy: () => boolean = serviceProxyArgument,
  engineMajor: () => number = dockerMajor,
): T & { 'userland-proxy-path'?: string } {
  if (configuration['userland-proxy'] !== true || 'userland-proxy-path' in configuration) return configuration
  if (engineMajor() < PROXY_RELOAD_VALIDATION_MAJOR) return configuration
  if (commandLineProxy())
    throw new Error(
      'Live gVisor registration is unavailable: Docker sets userland-proxy-path by command line',
    )
  const proxy =
    DOCKER_LIBEXEC_DIRECTORIES.map((directory) => find(join(directory, 'docker-proxy'))).find(Boolean) ??
    find('docker-proxy')
  if (!proxy)
    throw new Error(
      'Docker enables userland-proxy but docker-proxy is unavailable; install the engine proxy binary',
    )
  return { ...configuration, 'userland-proxy-path': proxy }
}

/** The pinned release's architecture is explicit whenever the server leaves the tier off. */
export function gvisorInstallProblem(arch: string): string | null {
  return arch === 'x64' ? null : `the pinned gVisor release supports x64; this host is ${arch}`
}

/** gVisor's runsc at deploy/gvisor.json's release, checked against its SHA-512, unpacked as is. */
export async function installGvisor(
  run: InstallCommand,
  prefix = join(GVISOR_PREFIX, gvisor.version),
  url = gvisor.url,
  release = gvisor,
) {
  const runsc = join(prefix, 'runsc')
  const installed = existsSync(runsc) ? Bun.spawnSync([runsc, '--version'], { stderr: 'ignore' }) : undefined
  if (installed?.exitCode === 0 && installed.stdout.toString().includes(`runsc version ${release.version}\n`))
    return verifyGvisor(run, prefix)
  const response = await fetch(url)
  if (!response.ok) throw new Error(`the gVisor release answered ${response.status}`)
  const bytes = new Uint8Array(await response.arrayBuffer())
  if (new Bun.CryptoHasher('sha512').update(bytes).digest('hex') !== release.sha512)
    throw new Error('the gVisor release does not match its pinned SHA-512')
  const staged = `${prefix}.new`
  rmSync(staged, { recursive: true, force: true })
  mkdirSync(staged, { recursive: true, mode: 0o755 })
  writeFileSync(join(staged, 'gvisor.tar'), Bun.zstdDecompressSync(bytes))
  await run(['tar', '-x', '--no-same-owner', '-f', join(staged, 'gvisor.tar'), '-C', staged])
  rmSync(join(staged, 'gvisor.tar'))
  await verifyGvisor(run, staged)
  rmSync(prefix, { recursive: true, force: true })
  renameSync(staged, prefix)
}

/** Check every pinned sidecar without fetching one or touching Docker's live configuration. */
async function verifyGvisor(run: InstallCommand, prefix: string) {
  const check = mkdtempSync(join(tmpdir(), 'pryx-gvisor-check-'))
  try {
    const config = join(check, 'daemon.json')
    writeFileSync(config, '{}\n')
    await run([
      join(prefix, 'runsc'),
      'install',
      '--config_file',
      config,
      '--download-sidecars=NEVER',
      '--require-sidecars=ALWAYS',
    ])
  } finally {
    rmSync(check, { recursive: true, force: true })
  }
}

/** Docker acknowledges a reload before the new runtime registration necessarily becomes
 * visible. Read the engine's exact selected path before admitting new sandboxes. */
export async function waitForGvisorRuntime(
  expected: string,
  read = async () => {
    const child = Bun.spawn(['docker', 'info', '--format', '{{json .Runtimes}}'], {
      stdout: 'pipe',
      stderr: 'ignore',
      timeout: SECOND_MS,
    })
    const text = await new Response(child.stdout).text()
    return (await child.exited) === 0 ? JSON.parse(text) : null
  },
  deadline = performance.now() + MINUTE_MS,
) {
  while (performance.now() < deadline) {
    try {
      if ((await read())?.runsc?.path === expected) return
    } catch {}
    await Bun.sleep(SECOND_MS)
  }
  throw new Error('Docker did not register the pinned gVisor executable; inspect its service journal')
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
  const unavailable = gvisorInstallProblem(process.arch)
  if (unavailable) console.error(`gvisor tier off: ${unavailable}`)
  else
    try {
      await installGvisor(run)
      runtimes = { runtimes: gvisorRuntimes(existing) }
    } catch (error) {
      console.error(`gvisor tier off: ${error instanceof Error ? error.message : error}`)
    }
  const configuration = { ...existing, ...daemonJson, ...runtimes }
  const desired = JSON.stringify(configuration, null, 2) + '\n'
  const current = existsSync('/etc/docker/daemon.json')
    ? readFileSync('/etc/docker/daemon.json', 'utf8')
    : null
  if (current !== desired) {
    const running = Bun.spawnSync(['docker', 'ps', '-q']).stdout.toString().trim()
    if (
      writeChanged(
        '/etc/docker/daemon.json',
        running ? JSON.stringify(dockerReloadConfiguration(configuration), null, 2) + '\n' : desired,
        0o644,
      )
    ) {
      if (running) await run(['systemctl', 'reload', 'docker'])
      else await run(['systemctl', 'restart', 'docker'])
    }
  }
  if (runtimes.runtimes) await waitForGvisorRuntime(join(GVISOR_PREFIX, gvisor.version, 'runsc'))
}
