// SPDX-License-Identifier: FSL-1.1-ALv2
/** The host installer can run the same release image under systemd. This operator path uses
 * the host engine and firewall; the image's ordinary process-tier start requires neither. */
import { existsSync, lstatSync, mkdirSync, readFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { MINUTE_MS, SECOND_MS } from '../../control/src/durations.ts'
import { PRYX_VERSION } from '../../control/src/generated/release.ts'
import { IMAGE_NODE_CONFIG } from '../../deploy/image-state.ts'
import nodeSchema from '../../openapi/node-writes.openapi.json'
import { callerHomeAndDropPrivileges, writeClientConfig } from './client-config.ts'
import { installBinary } from './up-assets.ts'
import { writeChanged } from './up-files.ts'
import { configureHostRuntime, type InstallCommand } from './up-host.ts'

const STATE = '/var/lib/pryx-image'
export const imageReference = (args: string[]) => {
  const next = args[args.indexOf('--image') + 1]
  const reference =
    next && !next.startsWith('--')
      ? next
      : (process.env.PRYX_IMAGE ?? `ghcr.io/cubepals/pryx:${PRYX_VERSION}`)
  if (!/^[a-zA-Z0-9][a-zA-Z0-9._/:@-]*$/.test(reference)) throw new Error('Invalid image reference')
  return reference
}
export function imageUnit(image: string) {
  return `[Unit]
Description=pryx release image
After=docker.service network-online.target
Requires=docker.service
[Service]
ExecStart=/usr/bin/docker run --rm --name pryx-image --network host --cap-add NET_ADMIN --env-file /etc/pryx/image.env --mount type=bind,source=${STATE},target=${STATE} --mount type=bind,source=/var/run/docker.sock,target=/var/run/docker.sock ${image}
ExecStop=/usr/bin/docker stop pryx-image
Restart=always
RestartSec=2
TimeoutStartSec=0
[Install]
WantedBy=multi-user.target
`
}
export async function upImage(args: string[], domain: string, run: InstallCommand) {
  for (const unit of ['pryx-control', 'pryxd'])
    if (Bun.spawnSync(['systemctl', 'is-active', '--quiet', unit]).exitCode === 0)
      throw new Error(
        'An existing binary installation is active; migrate its state before selecting the image installer',
      )
  await configureHostRuntime(run)
  for (const path of ['/etc/pryx', STATE]) {
    if (existsSync(path)) {
      const metadata = lstatSync(path)
      if (!metadata.isDirectory() || metadata.uid !== 0 || metadata.mode & 0o022)
        throw new Error('Unsafe image installation directory')
    }
    mkdirSync(path, { recursive: true, mode: path === STATE ? 0o711 : 0o700 })
  }
  const image = imageReference(args)
  if (
    Bun.spawnSync(['docker', 'image', 'inspect', image], { stdout: 'ignore', stderr: 'ignore' }).exitCode !==
    0
  )
    await run(['docker', 'pull', image])
  const created = Bun.spawnSync(['docker', 'create', image], { stdout: 'pipe', stderr: 'pipe' })
  if (created.exitCode) throw new Error('Cannot inspect the release image')
  const id = created.stdout.toString().trim()
  try {
    const source = join('/etc/pryx', 'image-agent')
    await run(['docker', 'cp', `${id}:/usr/local/lib/pryx/pryx-agent`, source])
    const target = nodeSchema['x-pryx-agent'].defaultBinary
    mkdirSync(dirname(target), { recursive: true, mode: 0o755 })
    installBinary(source, target)
    const daemon = join('/etc/pryx', 'image-pryxd')
    await run(['docker', 'cp', `${id}:/pryxd`, daemon])
    installBinary(daemon, '/usr/local/lib/pryx/pryxd-host')
  } finally {
    await run(['docker', 'rm', '-v', id])
  }
  const url = 'http://127.0.0.1:8080'
  const environment = {
    PRYX_STATE_DIR: STATE,
    PRYX_PUBLIC_DOMAIN: domain,
    PRYX_PUBLIC_URL: process.env.PRYX_PUBLIC_URL ?? url,
    ...(Object.fromEntries(
      Object.entries(process.env).filter(
        ([key, value]) =>
          value !== undefined &&
          (key === 'DATABASE_URL' || /^S3_(ENDPOINT|BUCKET|ACCESS_KEY|SECRET_KEY)$/.test(key)),
      ),
    ) as Record<string, string>),
  }
  for (const value of Object.values(environment))
    if (/[\r\n]/.test(value)) throw new Error('Image environment values must fit one line')
  const environmentChanged = writeChanged(
    '/etc/pryx/image.env',
    `${Object.entries(environment)
      .map(([key, value]) => `${key}=${value}`)
      .join('\n')}\n`,
    0o600,
  )
  const changed = writeChanged('/etc/systemd/system/pryx-image.service', imageUnit(image), 0o644)
  await run(['systemctl', 'daemon-reload'])
  const active = Bun.spawnSync(['systemctl', 'is-active', '--quiet', 'pryx-image']).exitCode === 0
  if (active && (changed || environmentChanged)) await run(['systemctl', 'restart', 'pryx-image'])
  else await run(['systemctl', 'enable', '--now', 'pryx-image'])
  const deadline = Date.now() + 2 * MINUTE_MS
  let ready = false
  while (Date.now() < deadline) {
    try {
      if ((await fetch(`${url}/healthz`)).ok) {
        ready = true
        break
      }
    } catch {}
    await Bun.sleep(SECOND_MS)
  }
  if (!ready) throw new Error('Release image did not become healthy; inspect its service journal')
  await run([
    '/usr/local/lib/pryx/pryxd-host',
    'reserve-ports',
    '--config',
    join(STATE, 'private', IMAGE_NODE_CONFIG),
  ])
  const privatePath = join(STATE, 'private', 'credentials.json')
  const metadata = lstatSync(privatePath)
  if (!metadata.isFile() || metadata.uid !== 0 || metadata.mode & 0o077)
    throw new Error('Unsafe image credentials')
  const key = (JSON.parse(readFileSync(privatePath, 'utf8')) as { key: string }).key
  const home = callerHomeAndDropPrivileges()
  const credentialPath = writeClientConfig({ url, key }, join(home, '.config/pryx/client.json'))
  console.log(JSON.stringify({ api: url, image, credentialPath }))
  console.log('pryx run --image alpine:3.22 -- echo hello')
}
