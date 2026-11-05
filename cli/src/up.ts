// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Idempotent Linux/systemd one-box installation. Check mode reads prerequisites and prints no credentials. */
import { chmodSync, existsSync, lstatSync, mkdirSync, readFileSync, readlinkSync, symlinkSync } from 'node:fs'
import { join } from 'node:path'
import { MINUTE_MS, SECOND_MS } from '../../control/src/durations.ts'
import { PRYX_VERSION } from '../../control/src/generated/release.ts'
import type { OperatorToken } from '../../control/src/operator-api.ts'
import { ensureBucket } from '../../control/src/store-bootstrap.ts'
import nodeSchema from '../../openapi/node-writes.openapi.json'
import { callerHomeAndDropPrivileges, writeClientConfig } from './client-config.ts'
import { option } from './sandbox.ts'
import { installBinary, releaseFiles } from './up-assets.ts'
import { configurations } from './up-config.ts'
import { readEnvironment, writeChanged, writeEnvironment } from './up-files.ts'
import { configureHostRuntime } from './up-host.ts'
import { container, service } from './up-units.ts'

export const runInstallCommand = async (args: string[], quiet = true, redactions: string[] = []) => {
  const p = Bun.spawn(args, {
    stdout: quiet ? 'pipe' : 'inherit',
    stderr: 'pipe',
    env: { ...process.env, DEBIAN_FRONTEND: 'noninteractive' },
  })
  const [stdout, stderr] = await Promise.all([
    quiet ? new Response(p.stdout).text() : Promise.resolve(''),
    new Response(p.stderr).text(),
  ])
  if ((await p.exited) !== 0) {
    let detail = `${stdout}\n${stderr}`
    for (const secret of redactions) detail = detail.replaceAll(secret, '[redacted]')
    throw new Error(`${args[0]} failed: ${detail.slice(-8192)}`)
  }
}
const active = (unit: string) => Bun.spawnSync(['systemctl', 'is-active', '--quiet', unit]).exitCode === 0
export function upPlan(domain: string) {
  return {
    version: PRYX_VERSION,
    domain,
    linux: process.platform === 'linux',
    systemd: existsSync('/run/systemd/system'),
    root: process.getuid?.() === 0,
    actions: [
      { item: 'Docker', needed: !Bun.which('docker') },
      { item: 'nftables', needed: !Bun.which('nft') },
      { item: 'release binaries and assets', needed: !existsSync('/opt/pryx/pryx-control') },
      { item: 'root-only unit environments', needed: !existsSync('/etc/pryx/control.env') },
      {
        item: 'Postgres and S3 volumes and units',
        needed: !existsSync('/etc/systemd/system/pryx-postgres.service'),
      },
      { item: 'control and edge units', needed: !existsSync('/etc/systemd/system/pryx-control.service') },
      { item: 'local pryxd enrollment', needed: !existsSync('/etc/pryxd/pryxd.toml') },
    ],
  }
}
async function until(work: () => Promise<boolean>, what: string) {
  const deadline = performance.now() + 2 * MINUTE_MS
  while (performance.now() < deadline) {
    try {
      if (await work()) return
    } catch {}
    await Bun.sleep(SECOND_MS)
  }
  throw new Error(`Timed out waiting for ${what}; inspect its service journal`)
}
export async function up(args: string[]) {
  const saved = args.includes('--check') ? {} : readEnvironment('/etc/pryx/control.env')
  const domain = option(args, '--domain') ?? saved.PRYX_PUBLIC_DOMAIN ?? 'localhost'
  if (saved.PRYX_PUBLIC_DOMAIN && saved.PRYX_PUBLIC_DOMAIN !== domain)
    throw new Error('The existing domain is immutable; migrate public address assignments before changing it')
  if (!/^[a-zA-Z0-9][a-zA-Z0-9.-]*$/.test(domain)) throw new Error('--domain is a DNS hostname')
  const plan = upPlan(domain)
  if (args.includes('--check')) return plan
  if (!plan.linux || !plan.systemd || !plan.root)
    throw new Error('up requires a Linux systemd host as root; use --check for a read-only plan')
  for (const [path, mode] of [
    ['/etc/pryx', 0o700],
    ['/opt/pryx', 0o755],
    ['/var/lib/pryxd', 0o700],
  ] as const) {
    if (existsSync(path)) {
      const entry = lstatSync(path)
      if (!entry.isDirectory() || entry.uid !== 0)
        throw new Error('A managed directory is not a root-owned directory')
    }
    mkdirSync(path, { recursive: true, mode })
    chmodSync(path, mode)
  }
  await configureHostRuntime(runInstallCommand)
  if (Bun.spawnSync(['id', '-u', 'pryx']).exitCode !== 0)
    await runInstallCommand([
      'useradd',
      '--system',
      '--home-dir',
      '/var/lib/pryx',
      '--create-home',
      '--shell',
      '/usr/sbin/nologin',
      'pryx',
    ])
  if (process.arch !== 'x64') throw new Error('This release installs Linux x86_64 binaries')
  const assets = await releaseFiles(option(args, '--from-dir'))
  for (const name of ['pryxd', 'pryx-agent', 'pryx-control', 'pryx-edge']) {
    chmodSync(join(assets, name), 0o755)
    const version = Bun.spawnSync([join(assets, name), '--version'])
    if (version.exitCode !== 0 || version.stdout.toString().trim() !== `${name} ${PRYX_VERSION}`)
      throw new Error('Release binary version mismatch')
  }
  mkdirSync('/opt/pryx', { recursive: true })
  const changed = new Set<string>()
  if (installBinary(join(assets, 'pryx-agent'), nodeSchema['x-pryx-agent'].defaultBinary))
    changed.add('pryxd')
  for (const name of ['pryx-control', 'pryx-edge'])
    if (installBinary(join(assets, name), `/opt/pryx/${name}`)) changed.add(name)
  mkdirSync('/var/lib/pryxd', { recursive: true, mode: 0o700 })
  chmodSync('/var/lib/pryxd', 0o700)
  if (installBinary(join(assets, 'pryxd'), '/var/lib/pryxd/bin/pryxd')) changed.add('pryxd')
  writeChanged('/var/lib/pryxd/bin/pryxd.sig', readFileSync(join(assets, 'pryxd.sig')), 0o644)
  const link = '/usr/local/bin/pryxd'
  if (
    !existsSync(link) ||
    (lstatSync(link).isSymbolicLink() && readlinkSync(link) !== '/var/lib/pryxd/bin/pryxd')
  ) {
    if (existsSync(link)) throw new Error('Existing pryxd is not the managed binary; move it before up')
    symlinkSync('/var/lib/pryxd/bin/pryxd', link)
  }
  const archive = join(assets, 'pryx-control-assets.tar.gz')
  const hash = await Bun.file(archive)
    .arrayBuffer()
    .then((b) => new Bun.CryptoHasher('sha256').update(b).digest('hex'))
  if (!existsSync('/opt/pryx/.assets.sha256') || readFileSync('/opt/pryx/.assets.sha256', 'utf8') !== hash) {
    await runInstallCommand(['tar', '-xzf', archive, '-C', '/opt/pryx'])
    writeChanged('/opt/pryx/.assets.sha256', hash, 0o644)
    changed.add('pryx-control')
  }
  const config = await configurations(domain)
  if (config.changed) changed.add('pryx-control')
  if (config.edgeChanged) changed.add('pryx-edge')
  const docker = Bun.which('docker') as string
  for (const name of ['postgres', 'store'] as const) {
    const inspect = Bun.spawnSync([
      docker,
      'inspect',
      '--format',
      '{{index .Config.Labels "pryx.service"}}',
      `pryx-${name}`,
    ])
    if (inspect.exitCode === 0 && inspect.stdout.toString().trim() !== name)
      throw new Error(`Container pryx-${name} is not owned by this installer`)
    if (Bun.spawnSync([docker, 'volume', 'inspect', `pryx-${name}`]).exitCode !== 0)
      await runInstallCommand([docker, 'volume', 'create', `pryx-${name}`])
    if (writeChanged(`/etc/systemd/system/pryx-${name}.service`, container(name, docker), 0o644))
      changed.add(`pryx-${name}`)
  }
  for (const name of ['control', 'edge'] as const)
    if (writeChanged(`/etc/systemd/system/pryx-${name}.service`, service(name), 0o644))
      changed.add(`pryx-${name}`)
  await runInstallCommand(['systemctl', 'daemon-reload'])
  for (const name of ['postgres', 'store', 'control', 'edge']) {
    const unit = `pryx-${name}`
    if (changed.has(unit) && active(unit)) await runInstallCommand(['systemctl', 'restart', unit])
    else await runInstallCommand(['systemctl', 'enable', '--now', unit])
  }
  const control = config.control
  await until(async () => {
    const r = await fetch('http://127.0.0.1:8081/fleet/v1/summary', {
      headers: { authorization: `Bearer ${control.PRYX_OPERATOR_TOKEN}` },
    })
    return r.ok
  }, 'control')
  await until(async () => {
    await ensureBucket({
      endpoint: control.S3_ENDPOINT as string,
      bucket: control.S3_BUCKET as string,
      accessKeyId: control.S3_ACCESS_KEY as string,
      secretAccessKey: control.S3_SECRET_KEY as string,
    })
    return true
  }, 'the S3 bucket')
  if (!existsSync('/etc/pryxd/pryxd.toml')) {
    const r = await fetch('http://127.0.0.1:8081/fleet/v1/tokens', {
      method: 'POST',
      headers: { authorization: `Bearer ${control.PRYX_OPERATOR_TOKEN}`, 'content-type': 'application/json' },
      body: JSON.stringify({ region: 'default' }),
    })
    if (!r.ok) throw new Error('Local enrollment token refused')
    const body = (await r.json()) as OperatorToken
    if (!('joinToken' in body) || typeof body.joinToken !== 'string' || !body.joinToken)
      throw new Error('The control service did not supply a local join token')
    const allow = args.flatMap((v, i) =>
      v === '--allow-image' && args[i + 1] ? [args[i + 1] as string] : [],
    )
    await runInstallCommand(
      [
        '/usr/local/bin/pryxd',
        'join',
        body.joinToken,
        '--address',
        '127.0.0.1',
        ...(allow.length ? allow : ['alpine:', 'node:']).flatMap((p) => ['--allow-image', p]),
      ],
      true,
      [body.token, body.joinToken],
    )
  }
  if (existsSync('/etc/pryxd/pryxd.toml')) {
    if (changed.has('pryxd') && active('pryxd')) await runInstallCommand(['systemctl', 'restart', 'pryxd'])
    else await runInstallCommand(['systemctl', 'enable', '--now', 'pryxd'])
  }
  await until(async () => {
    const r = await fetch('http://127.0.0.1:8081/fleet/v1/nodes', {
      headers: { authorization: `Bearer ${control.PRYX_OPERATOR_TOKEN}` },
    })
    const b = (await r.json()) as { nodes: Array<{ health: string }> }
    return b.nodes.some((n) => n.health === 'healthy')
  }, 'the local node heartbeat')
  const firstKey = !control.PRYX_BOOTSTRAP_PRINTED
  if (firstKey) {
    control.PRYX_BOOTSTRAP_PRINTED = '1'
    writeEnvironment('/etc/pryx/control.env', control)
  }
  const report = {
    version: PRYX_VERSION,
    domain,
    api: 'http://127.0.0.1:8082',
    units: ['pryx-postgres', 'pryx-store', 'pryx-control', 'pryx-edge', 'pryxd'],
    undo: [
      'systemctl disable --now pryx-edge pryx-control pryxd pryx-store pryx-postgres',
      'Remove /etc/systemd/system/pryx-*.service and run systemctl daemon-reload',
      'Remove /opt/pryx and /etc/pryx after saving credentials/backups',
      'Explicit data removal: docker volume rm pryx-postgres pryx-store; remove /var/lib/pryxd only after saving sandbox data',
      'Docker and nftables remain installed; Docker daemon.json remains configured',
    ],
  }
  const home = callerHomeAndDropPrivileges()
  const credentialPath = writeClientConfig(
    { url: report.api, key: control.PRYX_BOOTSTRAP_API_KEY as string },
    join(home, '.config/pryx/client.json'),
  )
  console.log(JSON.stringify({ ...report, credentialPath }, null, 2))
  console.log(`export PRYX_API_URL=${report.api}`)
  console.log(
    firstKey
      ? `export PRYX_API_KEY=${control.PRYX_BOOTSTRAP_API_KEY}`
      : 'export PRYX_API_KEY=$(pryx config key)',
  )
  console.log(`export PRYX_PUBLIC_DOMAIN=${domain}`)
  console.log('pryx run --image alpine:3.22 -- echo hello')
}
