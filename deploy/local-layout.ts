// SPDX-License-Identifier: FSL-1.1-ALv2
/** Locate the same supervisor's programs and private listeners in an image or host bundle.
 * Allocation does not promise a sandbox boundary; authenticated readiness still proves services. */
import { readFileSync } from 'node:fs'
import { createServer, type Server } from 'node:net'
import { join } from 'node:path'

const IMAGE_PORTS = { fleet: 8443, operator: 8081, control: 8082, node: 7443, ops: 7070, store: 9000 }
async function availablePorts() {
  const held: Server[] = []
  try {
    const ports = { ...IMAGE_PORTS }
    for (const name of Object.keys(ports) as Array<keyof typeof ports>) {
      const server = createServer()
      held.push(server)
      await new Promise<void>((resolve, reject) => {
        server.once('error', reject)
        server.listen(0, '127.0.0.1', resolve)
      })
      const address = server.address()
      if (!address || typeof address === 'string') throw new Error('Private TCP listener allocation failed')
      ports[name] = address.port
    }
    return ports
  } finally {
    await Promise.all(held.map((server) => new Promise<void>((resolve) => server.close(() => resolve()))))
  }
}
export function privateDirectory(root: string) {
  return process.env.PRYX_LOCAL_CONFIG_DIR ?? join(root, 'private')
}
export function bundleBinary(name: string, fallback: string) {
  return process.env.PRYX_BUNDLE_DIR ? join(process.env.PRYX_BUNDLE_DIR, 'bin', name) : fallback
}
export function postgresBinary(name: string) {
  return join(
    process.env.PRYX_BUNDLE_DIR
      ? join(process.env.PRYX_BUNDLE_DIR, 'postgres/bin')
      : '/usr/lib/postgresql/17/bin',
    name,
  )
}
export function embeddedUser() {
  if (process.env.PRYX_BUNDLE_DIR) {
    const metadata = JSON.parse(readFileSync(join(process.env.PRYX_BUNDLE_DIR, 'bundle.json'), 'utf8')) as {
      serviceUid: number
      serviceGid: number
    }
    if (
      !Number.isSafeInteger(metadata.serviceUid) ||
      metadata.serviceUid <= 0 ||
      !Number.isSafeInteger(metadata.serviceGid) ||
      metadata.serviceGid <= 0
    )
      throw new Error('Embedded PostgreSQL requires a non-root service identity')
    return { uid: metadata.serviceUid, gid: metadata.serviceGid }
  }
  const entry = readFileSync('/etc/passwd', 'utf8')
    .split('\n')
    .find((line) => line.startsWith('postgres:'))
  if (!entry) throw new Error('The embedded service user is missing')
  const fields = entry.split(':')
  return { uid: Number(fields[2]), gid: Number(fields[3]) }
}
export function stateRoot(base: string, local: boolean, uid = process.getuid?.()) {
  return local || uid === 0 ? base : join(base, `uid-${uid}`)
}
export async function localLayout() {
  const local = Boolean(process.env.PRYX_LOCAL_PRESET)
  const base = process.env.PRYX_STATE_DIR ?? '/var/lib/pryx'
  const root = stateRoot(base, local)
  return {
    local,
    root,
    private: privateDirectory(root),
    runtime: process.env.PRYX_RUNTIME_DIR ?? root,
    host: local && process.env.PRYX_LOCAL_PRESET === 'user' ? '127.0.0.1' : '0.0.0.0',
    ports: local ? await availablePorts() : { ...IMAGE_PORTS },
    pryxd: bundleBinary('pryxd', '/pryxd'),
    agent: bundleBinary('pryx-agent', '/usr/local/lib/pryx/pryx-agent'),
    agentos:
      process.env.PRYX_AGENTOS_DISTRIBUTION ??
      (process.env.PRYX_BUNDLE_DIR
        ? join(process.env.PRYX_BUNDLE_DIR, 'agentos')
        : '/usr/local/lib/pryx/agentos'),
  }
}
