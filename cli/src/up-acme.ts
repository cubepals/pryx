// SPDX-License-Identifier: FSL-1.1-ALv2
/** Install the pinned TLS service, retain its private state, and verify HTTPS through the edge. */
import { chmodSync, lstatSync, mkdirSync, mkdtempSync, readFileSync, rmSync } from 'node:fs'
import { get } from 'node:https'
import { dirname, join } from 'node:path'
import { StatusCodes } from 'http-status-codes'
import { SECOND_MS } from '../../control/src/durations.ts'
import { TLS_FRONT_PORT } from '../../control/src/edge-headers.ts'
import { API_TLS_UNIT } from '../../control/src/managed-api.ts'
import caddy from '../../deploy/caddy.json'
import {
  API_TLS_BIN,
  API_TLS_CONFIG,
  caddyConfig,
  PEBBLE_DIRECTORY_FILE,
  PEBBLE_ISSUER_FILE,
  PEBBLE_ROOT_FILE,
  type pebbleOptions,
} from './up-acme-config.ts'
import { writeChanged } from './up-files.ts'
import type { InstallCommand } from './up-host.ts'

const hash = (data: Uint8Array) => new Bun.CryptoHasher('sha512').update(data).digest('hex')

async function installCaddy(run: InstallCommand) {
  const release = process.arch === 'x64' ? caddy.x64 : process.arch === 'arm64' ? caddy.arm64 : null
  if (!release) throw new Error('Managed API TLS supports x86_64 and aarch64')
  const installed = lstatSync(API_TLS_BIN, { throwIfNoEntry: false })
  if (installed && (!installed.isFile() || installed.uid !== 0 || installed.mode & 0o022))
    throw new Error('Unsafe managed API TLS binary')
  if (installed && hash(readFileSync(API_TLS_BIN)) === release.binarySha512) {
    chmodSync(API_TLS_BIN, 0o755)
    return false
  }

  const stage = mkdtempSync(join(dirname(API_TLS_BIN), '.caddy-'))
  try {
    const archive = join(stage, 'caddy.tar.gz')
    const url = `https://github.com/caddyserver/caddy/releases/download/v${caddy.version}/caddy_${caddy.version}_linux_${release.machine}.tar.gz`
    await run([
      'curl',
      '--fail',
      '--silent',
      '--show-error',
      '--location',
      '--proto',
      '=https',
      '--proto-redir',
      '=https',
      '--retry',
      '3',
      '--connect-timeout',
      '15',
      '--max-time',
      '120',
      '--output',
      archive,
      url,
    ])
    if (hash(readFileSync(archive)) !== release.archiveSha512)
      throw new Error('The Caddy archive does not match its pinned SHA-512')
    await run(['tar', '-xzf', archive, '-C', stage, '--', 'caddy'])
    const binary = readFileSync(join(stage, 'caddy'))
    if (hash(binary) !== release.binarySha512)
      throw new Error('The Caddy binary does not match its pinned SHA-512')
    return writeChanged(API_TLS_BIN, binary, 0o755)
  } finally {
    rmSync(stage, { recursive: true, force: true })
  }
}

export async function configureApiTls(
  domain: string,
  pebble: ReturnType<typeof pebbleOptions>,
  run: InstallCommand,
) {
  const entry = lstatSync(API_TLS_CONFIG, { throwIfNoEntry: false })
  if (entry && (!entry.isDirectory() || entry.uid !== 0 || entry.mode & 0o022))
    throw new Error('Unsafe managed API TLS configuration directory')
  mkdirSync(API_TLS_CONFIG, { recursive: true, mode: 0o755 })
  chmodSync(API_TLS_CONFIG, 0o755)
  const binaryChanged = await installCaddy(run)
  if (pebble) {
    writeChanged(join(API_TLS_CONFIG, PEBBLE_DIRECTORY_FILE), `${pebble.directory}\n`, 0o600)
    writeChanged(join(API_TLS_CONFIG, PEBBLE_ROOT_FILE), pebble.root, 0o600)
  }
  const trustChanged = pebble
    ? writeChanged(join(API_TLS_CONFIG, PEBBLE_ISSUER_FILE), pebble.issuer, 0o644)
    : false
  const configChanged = writeChanged(
    join(API_TLS_CONFIG, 'caddy.json'),
    `${JSON.stringify(caddyConfig(domain, pebble), null, 2)}\n`,
    0o644,
  )
  const unitChanged = writeChanged(
    `/etc/systemd/system/${API_TLS_UNIT}.service`,
    `[Unit]
Description=pryx API TLS
After=network-online.target
Wants=network-online.target
[Service]
Type=notify
DynamicUser=yes
Group=pryx
RuntimeDirectory=${API_TLS_UNIT}
RuntimeDirectoryMode=0750
StateDirectory=${API_TLS_UNIT}
StateDirectoryMode=0700
UMask=0077
Environment=XDG_DATA_HOME=/var/lib/${API_TLS_UNIT}
Environment=XDG_CONFIG_HOME=/var/lib/${API_TLS_UNIT}
ExecStart=${API_TLS_BIN} run --config ${API_TLS_CONFIG}/caddy.json
Restart=on-failure
NoNewPrivileges=yes
ProtectSystem=strict
ProtectHome=yes
PrivateTmp=yes
PrivateDevices=yes
RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6
CapabilityBoundingSet=
[Install]
WantedBy=multi-user.target
`,
    0o644,
  )
  return binaryChanged || trustChanged || configChanged || unitChanged
}

export function apiTlsReady(domain: string, ca?: string) {
  return new Promise<boolean>((resolve) => {
    const request = get(
      {
        host: '127.0.0.1',
        port: TLS_FRONT_PORT,
        servername: domain,
        headers: { host: domain },
        path: '/v1/sandboxes',
        ca,
        rejectUnauthorized: true,
        agent: false,
        signal: AbortSignal.timeout(2 * SECOND_MS),
      },
      (response) => {
        response.resume()
        resolve(response.statusCode === StatusCodes.UNAUTHORIZED)
      },
    )
    request.once('error', () => resolve(false))
  })
}
