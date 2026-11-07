// SPDX-License-Identifier: FSL-1.1-ALv2
/** Real wasm-specific placement, declared HTTP ingress and prepared-template acceptance. The
 * shared K11 suite separately covers both SDKs' complete agent surface on this same backend. */
import { expect, test } from 'bun:test'
import { StatusCodes } from 'http-status-codes'
import { MINUTE_MS } from '../src/durations.ts'
import { agentDeployment } from './agent-context.ts'
import { until } from './waits.ts'

test(
  'wasm declared ports and templates use real VMs and edge routes',
  async () => {
    const c = await agentDeployment('wasm')
    const box = await c.sdk.sandbox({
      isolation: 'wasm',
      packages: ['coreutils'],
      ports: [{ name: 'web', port: 3000, protocol: 'tcp', audience: 'public', front: 'http' }],
      entrypoint: [
        'node',
        '-e',
        "require('node:http').createServer((req,res)=>res.end('declared')).listen(3000,'0.0.0.0')",
      ],
      resources: { memoryMb: 256 },
      sparePorts: 0,
    })
    try {
      expect(box.isolation).toBe('wasm')
      expect(box.units).toBe(1)
      expect((await box.run('id -u')).stdout.trim()).not.toBe('0')
      await box.write('from-api', 'owned')
      expect((await box.run('cat /data/from-api')).stdout).toBe('owned')
      expect((await box.run('printf guest-updated > /data/from-api')).code).toBe(0)
      expect(await box.read('from-api')).toBe('guest-updated')
      const endpoint = (await c.sdk.api.get(box.id)).endpoints.find((port) => port.name === 'web')
      if (!endpoint?.hostname) throw new Error('Declared port has no public address')
      await until('declared wasm HTTP ingress', async () => {
        const response = await c.app(`http://${endpoint.hostname}`)
        return response.status === StatusCodes.OK && (await response.text()) === 'declared' ? true : null
      })
      await expect(c.sdk.sandbox({ image: 'alpine:3.22', isolation: 'wasm' })).rejects.toMatchObject({
        status: StatusCodes.UNPROCESSABLE_ENTITY,
      })
      const template = await c.sdk.api.template({
        name: 'wasm-prepared',
        spec: { isolation: 'wasm', packages: ['coreutils'] },
      })
      const prepared = template as { id: string; copies: Array<{ ready: boolean }> }
      expect(prepared.copies.some((copy) => copy.ready)).toBe(true)
      const fork = await c.sdk.sandbox({ template: prepared.id })
      try {
        expect(fork.isolation).toBe('wasm')
        expect((await fork.run('printf template-ready')).stdout).toBe('template-ready')
      } finally {
        await fork.delete()
      }
    } finally {
      await box.delete()
      await c.close()
    }
  },
  4 * MINUTE_MS,
)
