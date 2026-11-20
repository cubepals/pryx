// SPDX-License-Identifier: FSL-1.1-ALv2
/** The named server's HTTPS contract and separate root-controlled Pebble fixture settings. */
import { X509Certificate } from 'node:crypto'
import { closeSync, constants, fstatSync, lstatSync, openSync, readFileSync } from 'node:fs'
import { isIP } from 'node:net'
import { isAbsolute, join } from 'node:path'
import { MANAGED_API_SOCKET, MANAGED_API_TLS_SOCKET } from '../../control/src/managed-api.ts'

export const API_TLS_CONFIG = '/etc/pryx-api-tls'
export const API_TLS_BIN = '/opt/pryx/caddy'
const ACME_SOLVER_PORT = 7343
export const PEBBLE_DIRECTORY_FILE = 'pebble-directory'
export const PEBBLE_ISSUER_FILE = 'pebble.pem'
export const PEBBLE_ROOT_FILE = 'pebble-root.pem'

export function dnsDomain(value: string) {
  const name = value.toLowerCase()
  if (name === 'localhost') return name
  if (
    name.length > 253 ||
    !name.includes('.') ||
    isIP(name) ||
    URL.parse(`https://${name}`)?.hostname !== name ||
    !name.split('.').every((label) => /^[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?$/.test(label))
  )
    throw new Error('--domain is a DNS hostname, such as sandboxes.example.com')
  return name
}

export function managedApiTls(domain: string, saved: Record<string, string>) {
  return domain !== 'localhost' && (!saved.PRYX_PUBLIC_DOMAIN || saved.PRYX_MANAGED_API_TLS === '1')
}

type Pebble = { directory: string; issuer: string; root: string }

function publicCert(path: string) {
  if (!isAbsolute(path)) throw new Error('Pebble trust requires absolute public certificate paths')
  const fd = openSync(path, constants.O_RDONLY | constants.O_NOFOLLOW | constants.O_NONBLOCK)
  try {
    const metadata = fstatSync(fd)
    if (!metadata.isFile() || metadata.size > 16 * 1024) throw new Error('Invalid Pebble public certificate')
    const pem = readFileSync(fd, 'utf8')
    if (!/^-----BEGIN CERTIFICATE-----\r?\n[A-Za-z0-9+/=\r\n]+-----END CERTIFICATE-----\s*$/.test(pem))
      throw new Error('Expected a single public Pebble certificate')
    return new X509Certificate(pem).toString()
  } finally {
    closeSync(fd)
  }
}

function pebbleDirectory(directory: string) {
  const url = URL.parse(directory)
  if (
    !url ||
    url.protocol !== 'https:' ||
    !['127.0.0.1', '[::1]'].includes(url.hostname) ||
    url.username ||
    url.password ||
    url.search ||
    url.hash
  )
    throw new Error('The Pebble directory must be a loopback HTTPS URL')
  return url.href
}

export function pebbleOptions(env = process.env, folder = API_TLS_CONFIG): Pebble | undefined {
  const { PRYX_ACME_TEST_DIRECTORY: directory, PRYX_ACME_TEST_CA: issuer, PRYX_ACME_TEST_ROOT: root } = env
  if (directory !== undefined || issuer !== undefined || root !== undefined) {
    if (!directory || !issuer || !root)
      throw new Error('Pebble requires a directory and both public CA files')
    return { directory: pebbleDirectory(directory), issuer: publicCert(issuer), root: publicCert(root) }
  }
  const folderEntry = lstatSync(folder, { throwIfNoEntry: false })
  if (!folderEntry) return
  if (!folderEntry.isDirectory() || folderEntry.uid !== 0 || folderEntry.mode & 0o022)
    throw new Error('Unsafe managed API TLS configuration directory')
  const saved = join(folder, PEBBLE_DIRECTORY_FILE)
  const entry = lstatSync(saved, { throwIfNoEntry: false })
  if (!entry) return
  if (!entry.isFile() || entry.uid !== 0 || entry.mode & 0o077)
    throw new Error('Unsafe saved Pebble directory')
  return {
    directory: pebbleDirectory(readFileSync(saved, 'utf8').trim()),
    issuer: publicCert(join(folder, PEBBLE_ISSUER_FILE)),
    root: publicCert(join(folder, PEBBLE_ROOT_FILE)),
  }
}

export function caddyConfig(domain: string, pebble?: Pebble) {
  const issuer = {
    module: 'acme',
    // Keep the certificate challenge on the same TCP/SNI path as client TLS.
    challenges: {
      http: { disabled: true },
      // CertMagic's auxiliary challenge listener stays on unprivileged loopback TCP.
      'tls-alpn': { alternate_port: ACME_SOLVER_PORT },
      bind_host: '127.0.0.1',
    },
    ...(pebble
      ? {
          ca: pebble.directory,
          test_ca: pebble.directory,
          trusted_roots_pem_files: [join(API_TLS_CONFIG, PEBBLE_ISSUER_FILE)],
        }
      : {}),
  }
  return {
    admin: { disabled: true },
    apps: {
      tls: {
        automation: {
          policies: [
            { subjects: [domain], issuers: [issuer], ...(pebble ? { renewal_window_ratio: 0.5 } : {}) },
          ],
          ...(pebble ? { renew_interval: '1s' } : {}),
        },
      },
      http: {
        servers: {
          api: {
            listen: [`unix/${MANAGED_API_TLS_SOCKET}|0660`],
            automatic_https: { disable_redirects: true },
            tls_connection_policies: [{ match: { sni: [domain] } }],
            strict_sni_host: true,
            protocols: ['h1', 'h2'],
            routes: [
              {
                match: [{ host: [domain] }],
                handle: [{ handler: 'reverse_proxy', upstreams: [{ dial: `unix/${MANAGED_API_SOCKET}` }] }],
              },
            ],
          },
        },
      },
    },
  }
}
