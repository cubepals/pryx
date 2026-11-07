// SPDX-License-Identifier: FSL-1.1-ALv2
/** Local image configuration compatibility and no-follow replacement. CI's real-image cohorts
 * cover service startup, VM admission and throughput. */
import { expect, test } from 'bun:test'
import {
  chmodSync,
  lstatSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  statSync,
  symlinkSync,
  writeFileSync,
} from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { imagePortRange } from '../../deploy/image-network.ts'
import { createPrivateFile } from '../../deploy/image-state.ts'
import nodeSchema from '../../openapi/node-reads.openapi.json'

test('image pools avoid outgoing ports and retain explicit and legacy restart addresses', () => {
  const dir = mkdtempSync(join(tmpdir(), 'pryx-image-ports-'))
  try {
    const path = join(dir, 'pryxd.toml')
    const selected = imagePortRange(path)
    const ephemeral = Number(readFileSync('/proc/sys/net/ipv4/ip_local_port_range', 'utf8').split(/\s+/)[0])
    expect(selected[1]).toBeLessThan(ephemeral)
    expect(selected[0]).toBeGreaterThanOrEqual(
      Number(readFileSync('/proc/sys/net/ipv4/ip_unprivileged_port_start', 'utf8')),
    )
    writeFileSync(path, `[network]\nport_range = ${JSON.stringify(selected)}\n`, { mode: 0o600 })
    expect(imagePortRange(path)).toEqual(selected)
    writeFileSync(path, '[network]\n')
    expect([...imagePortRange(path)]).toEqual(nodeSchema['x-pryx-network'].defaultPortRange)
    chmodSync(path, 0o644)
    expect(() => imagePortRange(path)).toThrow('Unsafe image node configuration')
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }
})

test('image pool configuration never follows a file symlink', () => {
  const dir = mkdtempSync(join(tmpdir(), 'pryx-image-ports-link-'))
  try {
    const path = join(dir, 'pryxd.toml'),
      target = join(dir, 'outside')
    writeFileSync(target, '[network]\n', { mode: 0o600 })
    symlinkSync(target, path)
    expect(() => imagePortRange(path)).toThrow()
    rmSync(target)
    expect(() => imagePortRange(path)).toThrow()
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }
})

test('private image configuration replacement is atomic and never writes through a symlink', () => {
  const dir = mkdtempSync(join(tmpdir(), 'pryx-image-config-'))
  try {
    const target = join(dir, 'outside'),
      path = join(dir, 'pryxd.toml')
    writeFileSync(target, 'outside', { mode: 0o600 })
    symlinkSync(target, path)
    createPrivateFile(path, 'first', true)
    expect(lstatSync(path).isSymbolicLink()).toBe(false)
    expect(readFileSync(target, 'utf8')).toBe('outside')
    expect(readFileSync(path, 'utf8')).toBe('first')
    createPrivateFile(path, 'second', true)
    expect(readFileSync(path, 'utf8')).toBe('second')
    expect(statSync(path).mode & 0o777).toBe(0o600)
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }
})
