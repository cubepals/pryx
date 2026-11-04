// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Provider wire requests use the published contracts; no credentials or real rented hosts are used. */

import { expect, test } from 'bun:test'
import { StatusCodes } from 'http-status-codes'
import { BoatProvider } from '../src/providers/boat.ts'
import { HetznerProvider } from '../src/providers/hetzner.ts'
import { ManualProvider } from '../src/providers/manual.ts'
import { privateIp } from '../src/providers/private-network.ts'
import type { Shape } from '../src/providers/provider.ts'

const shape: Shape = {
  name: 'small',
  cpus: 4,
  memoryGb: 8,
  diskGb: 40,
  price: { currency: 'TEST', hour: '0', month: '0' },
}
test('Hetzner attaches the explicit network/firewall and sends bootstrap only in cloud-init', async () => {
  const calls: Array<{ path: string; body: Record<string, unknown> }> = []
  const send = (async (input, options) => {
    const path = new URL(String(input)).pathname
    const body = options?.body ? JSON.parse(String(options.body)) : {}
    calls.push({ path, body })
    if (options?.method === 'POST')
      return Response.json(
        {
          server: {
            id: 1,
            location: { name: 'test-region' },
            private_net: [{ network: 2, ip: '10.0.0.8' }],
            server_type: { name: 'small' },
            labels: { 'pryx.acquire': 'request' },
          },
        },
        { status: StatusCodes.CREATED },
      )
    return Response.json({ servers: [], meta: { pagination: { next_page: null } } })
  }) as typeof fetch
  const p = new HetznerProvider({
    token: 'test-provider-token',
    deployment: 'test',
    network: 2,
    firewall: 3,
    sshKeys: ['test-key'],
    image: 'ubuntu-24.04',
    send,
  })
  const host = await p.acquire('test-region', shape, 'request', {
    line: 'echo JOIN_SECRET',
    allowedImages: ['alpine:'],
  })
  expect(host.address).toBe('10.0.0.8')
  const body = calls.find((c) => c.body.user_data)?.body
  expect(body?.networks).toEqual([2])
  expect(body?.firewalls).toEqual([{ firewall: 3 }])
  expect(body?.user_data).toContain('JOIN_SECRET')
  expect(JSON.stringify(body?.labels)).not.toContain('JOIN_SECRET')
})
test('Boat keeps bootstrap contents out of command arguments and uses native creation identity', async () => {
  const commands: string[] = [],
    files: string[] = [],
    headers: Headers[] = []
  const send = (async (input, options) => {
    const url = new URL(String(input)),
      body = options?.body ? JSON.parse(String(options.body)) : {}
    if (url.pathname.endsWith('/commands')) {
      commands.push(body.command)
      return Response.json({
        exitCode: 0,
        stdout: body.command === 'private-address' ? '10.0.0.8\n' : '',
        stderr: '',
        success: true,
        timedOut: false,
      })
    }
    if (url.pathname.endsWith('/files')) {
      files.push(body.content)
      return Response.json({ success: true })
    }
    if (options?.method === 'POST') {
      headers.push(new Headers(options.headers))
      expect(body.noEnv).toBe(true)
      return Response.json({
        sandbox: { id: 'bx_23456789', name: 'test', type: 'small', state: 'ready', labels: body.labels },
      })
    }
    return Response.json({ sandboxes: [], pageInfo: { hasMore: false } })
  }) as typeof fetch
  const p = new BoatProvider({
    token: 'test-provider-token',
    deployment: 'test',
    region: 'logical',
    catalog: [shape],
    networkSetup: 'echo network',
    privateAddressCommand: 'private-address',
    send,
  })
  const host = await p.acquire('logical', shape, 'request', {
    line: 'echo JOIN_SECRET',
    allowedImages: ['alpine:'],
  })
  expect(host.address).toBe('10.0.0.8')
  expect(headers[0]?.get('Idempotency-Key')).toBe('request')
  expect(files.some((f) => f.includes('JOIN_SECRET'))).toBe(true)
  expect(commands.some((c) => c.includes('JOIN_SECRET'))).toBe(false)
})
test('manual SSH preserves host verification and keeps join bytes on stdin', async () => {
  let args: string[] = [],
    stdin = ''
  const p = new ManualProvider({
    address: '10.0.0.8',
    region: 'default',
    shape,
    run: async (a, input) => {
      args = a
      stdin = input
      return ''
    },
  })
  await p.acquire('default', shape, 'request', { line: 'echo JOIN_SECRET', allowedImages: ['alpine:'] })
  expect(args).toContain('StrictHostKeyChecking=yes')
  expect(args.join(' ')).not.toContain('JOIN_SECRET')
  expect(stdin).toContain('JOIN_SECRET')
  expect(p.managed).toBe(false)
})
test('shared and private address classes do not admit unrelated public space', () => {
  expect(privateIp('100.64.0.1')).toBe(true)
  expect(privateIp('100.1.0.1')).toBe(false)
  expect(privateIp('172.31.1.2')).toBe(true)
  expect(privateIp('172.32.1.2')).toBe(false)
  expect(privateIp('fd12::1')).toBe(true)
  expect(privateIp('2001:db8::1')).toBe(false)
})
