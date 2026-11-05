// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Token-only live rental test. Temporary network/firewall/SSH resources and strict tunnels isolate the CI control plane. */
import { describe, expect, test } from 'bun:test'
import { execFileSync } from 'node:child_process'
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { setTimeout as pause } from 'node:timers/promises'
import { Pryx } from '@cubepals/pryx'
import { sql } from 'drizzle-orm'
import { StatusCodes } from 'http-status-codes'
import { smallestShape } from '../src/autoscale.ts'
import { FleetCa } from '../src/ca.ts'
import { loadConfig } from '../src/config.ts'
import { MINUTE_MS, SECOND_MS } from '../src/durations.ts'
import { createKey } from '../src/keys.ts'
import { startControl } from '../src/main.ts'
import { ProviderClient } from '../src/providers/client.ts'
import type { paths } from '../src/providers/generated/hetzner.ts'
import { HetznerProvider } from '../src/providers/hetzner.ts'
import { one } from '../src/sql.ts'
import { freshDatabase } from '../test/nodes.ts'
import { createTestBucket } from './test-bucket.ts'

type Created<P extends keyof paths> = paths[P] extends {
  post: { responses: { 201: { content: { 'application/json': infer R } } } }
}
  ? R
  : never
const token = process.env.PRYX_HETZNER_TOKEN
if (!token) console.info('SKIP Hetzner live test: PRYX_HETZNER_TOKEN is absent')
describe.skipIf(!token)('Hetzner live provider', () => {
  test(
    'CLI rents, enrolls, places and returns one host',
    async () => {
      const dir = mkdtempSync(`${tmpdir()}/pryx-hcloud-`),
        name = `pryx-e2e-${crypto.randomUUID().slice(0, 8)}`
      const api = new ProviderClient('https://api.hetzner.cloud/v1', token ?? '')
      const fresh = await freshDatabase(),
        pems = await FleetCa.generate(name)
      let control: Awaited<ReturnType<typeof startControl>> | undefined, tunnel: Bun.Subprocess | undefined
      let network: number | undefined, firewall: number | undefined, sshKey: number | undefined
      let acquisition: string | undefined, serverId: string | undefined
      const region = 'fsn1' // Test location's network zone is eu-central, per Hetzner's location catalog.
      const wait = async <T>(label: string, run: () => Promise<T | null>, minutes = 10): Promise<T> => {
        const deadline = performance.now() + minutes * MINUTE_MS
        while (performance.now() < deadline) {
          const result = await run()
          if (result) return result
          await pause(SECOND_MS)
        }
        throw new Error(`Timed out: ${label}`)
      }
      try {
        for (const key of ['client', 'host'])
          execFileSync('ssh-keygen', ['-q', '-t', 'ed25519', '-N', '', '-C', name, '-f', `${dir}/${key}`])
        const runnerIp = (await (await fetch('https://api.ipify.org')).text()).trim()
        if (!/^\d+\.\d+\.\d+\.\d+$/.test(runnerIp)) throw new Error('CI public IPv4 unavailable')
        const n = await api.request<Created<'/networks'>>('/networks', 'POST', {
          name,
          ip_range: '10.83.0.0/16',
          subnets: [{ type: 'cloud', ip_range: '10.83.1.0/24', network_zone: 'eu-central' }],
        })
        if (!n.network) throw new Error('Network creation did not return an id')
        network = n.network.id
        const f = await api.request<Created<'/firewalls'>>('/firewalls', 'POST', {
          name,
          rules: [{ direction: 'in', protocol: 'tcp', port: '22', source_ips: [`${runnerIp}/32`] }],
        })
        if (!f.firewall) throw new Error('Firewall creation did not return an id')
        firewall = f.firewall.id
        const k = await api.request<Created<'/ssh_keys'>>('/ssh_keys', 'POST', {
          name,
          public_key: readFileSync(`${dir}/client.pub`, 'utf8'),
        })
        sshKey = k.ssh_key.id
        const hostKey = readFileSync(`${dir}/host`, 'utf8'),
          hostPublic = readFileSync(`${dir}/host.pub`, 'utf8').trim()
        const provider = new HetznerProvider({
          token: token ?? '',
          deployment: name,
          network,
          firewall,
          sshKeys: [String(sshKey)],
          image: 'ubuntu-24.04',
          joinAddress: '127.0.0.1',
          userDataPrefix: `printf %s '${Buffer.from(hostKey).toString('base64')}' | base64 -d > /etc/ssh/ssh_host_ed25519_key\nchmod 0600 /etc/ssh/ssh_host_ed25519_key\nsystemctl restart ssh\n`,
        })
        const s3 = {
          endpoint: process.env.S3_TEST_ENDPOINT ?? 'http://127.0.0.1:9000',
          bucket: name,
          accessKeyId: 'blockly-test',
          secretAccessKey: 'blockly-test-secret',
        }
        await createTestBucket(s3.endpoint, s3, name)
        const op = crypto.randomUUID(),
          config = loadConfig({
            DATABASE_URL: fresh.url,
            PRYX_DEPLOYMENT: name,
            PRYX_CA_CERT: pems.certPem,
            PRYX_CA_KEY: pems.keyPem,
            PRYX_OPERATOR_TOKEN: op,
            PRYX_NODE_LISTEN: '127.0.0.1:8443',
            PRYX_PROVIDER_ALLOW_IMAGES: 'alpine:',
            PRYX_REGION_MIN_NODES: '0',
            PRYX_PRYXD_BIN: process.env.PRYXD_STATIC_BIN,
            S3_ENDPOINT: s3.endpoint,
            S3_BUCKET: name,
            S3_ACCESS_KEY: s3.accessKeyId,
            S3_SECRET_KEY: s3.secretAccessKey,
          })
        control = await startControl(
          { ...config, operatorListen: '127.0.0.1:0', apiListen: '127.0.0.1:0', heartbeatSeconds: 1 },
          { providers: new Map([['hetzner', provider]]) },
        )
        const current = control
        const cli = async (args: string[]) => {
          const child = Bun.spawn(['bun', 'cli/src/main.ts', 'node', ...args], {
            env: {
              ...process.env,
              PRYX_OPERATOR_API: current.operatorUrl.replace(/\/$/, ''),
              PRYX_OPERATOR_TOKEN: op,
            },
            stdout: 'pipe',
            stderr: 'pipe',
          })
          const output = await new Response(child.stdout).text()
          await new Response(child.stderr).text()
          expect(await child.exited).toBe(0)
          return JSON.parse(output)
        }
        const catalog = await cli(['shapes', region, '--provider', 'hetzner'])
        const shape = smallestShape(catalog.shapes, { memoryMb: 128, cpuMillis: 32, diskGb: 1 })
        if (!shape) throw new Error('No fitting x86 server available')
        const made = await cli([
          'acquire',
          region,
          '--provider',
          'hetzner',
          '--shape',
          shape.name,
          '--idempotency-key',
          name,
        ])
        acquisition = made.id
        const host = await wait('provider id', async () => (await current.hosts.record(made.id)).host)
        serverId = host.id
        const server = await api.request<
          paths['/servers/{id}']['get']['responses'][200]['content']['application/json']
        >(`/servers/${serverId}`)
        const ip = server.server?.public_net.ipv4?.ip
        if (!ip) throw new Error('No SSH address')
        writeFileSync(`${dir}/known_hosts`, `${ip} ${hostPublic}\n`, { mode: 0o600 })
        const ssh = [
          'ssh',
          '-i',
          `${dir}/client`,
          '-o',
          'BatchMode=yes',
          '-o',
          'StrictHostKeyChecking=yes',
          '-o',
          `UserKnownHostsFile=${dir}/known_hosts`,
          '-o',
          'ConnectTimeout=5',
        ]
        await wait('SSH', async () => {
          const p = Bun.spawn([...ssh, `root@${ip}`, 'true'], { stdout: 'ignore', stderr: 'ignore' })
          return (await p.exited) === 0 ? true : null
        })
        tunnel = Bun.spawn(
          [
            ...ssh,
            '-N',
            '-o',
            'ExitOnForwardFailure=yes',
            '-o',
            'ServerAliveInterval=10',
            '-R',
            '127.0.0.1:8443:127.0.0.1:8443',
            '-R',
            '127.0.0.1:9000:127.0.0.1:9000',
            '-L',
            '127.0.0.1:7443:127.0.0.1:7443',
            `root@${ip}`,
          ],
          { stdout: 'ignore', stderr: 'ignore' },
        )
        const joined = await wait('first healthy heartbeat', async () => {
          const a = await current.hosts.record(made.id)
          return a.ready_at && a.node_id ? a : null
        })
        const measurement = await one<{ seconds: number }>(
          current.db,
          sql`SELECT extract(epoch FROM ready_at-requested_at)::float8 AS seconds FROM provider_acquisitions WHERE id=${made.id}`,
        )
        console.info(
          JSON.stringify({ provider: 'hetzner', n: 1, acquireToHealthySeconds: measurement?.seconds }),
        )
        await current.runtime.refreshRegistry()
        const key = await createKey(current.db, 'default', 'provider-e2e'),
          sdk = new Pryx({ baseUrl: current.apiUrl, apiKey: key.key })
        const box = await sdk.make({
          region,
          image: 'alpine:3.22',
          resources: { memoryMb: 128 },
          entrypoint: ['sh', '-c', 'trap "exit 0" TERM; while :; do sleep 1; done'],
        })
        try {
          expect(
            await Array.fromAsync(sdk.exec(box.id, { command: ['echo', 'provider-live'] })),
          ).toContainEqual(expect.objectContaining({ type: 'stdout', data: 'provider-live\n' }))
        } finally {
          await sdk.delete(box.id)
        }
        expect((await cli(['release', joined.node_id ?? ''])).state).toBe('released')
        expect((await provider.list()).some((h) => h.id === serverId)).toBe(false)
        serverId = undefined
      } finally {
        // Only test-created resources are removed. A failed bootstrap must not leak a paid server.
        if (control && acquisition) {
          const record = await control.hosts.record(acquisition).catch(() => null)
          serverId ??= record?.host?.id
        }
        if (serverId) {
          await api.request(`/servers/${serverId}`, 'DELETE')
          await wait(
            'cleanup delete',
            async () => {
              try {
                await api.request(`/servers/${serverId}`)
                return null
              } catch (error) {
                return (error as { status?: number }).status === StatusCodes.NOT_FOUND ? true : null
              }
            },
            3,
          )
        }
        tunnel?.kill()
        if (tunnel) await tunnel.exited
        await control?.stop()
        for (const [path, id] of [
          ['firewalls', firewall],
          ['networks', network],
          ['ssh_keys', sshKey],
        ] as const)
          if (id) await api.request(`/${path}/${id}`, 'DELETE')
        await fresh.drop()
        rmSync(dir, { recursive: true, force: true })
      }
    },
    20 * MINUTE_MS,
  )
})
