// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/**
 * Token-only live rental test. Temporary network/firewall/SSH resources and strict tunnels isolate the CI
 * control plane. On failure it prints the rented box's and control's own account before cleanup, and the
 * first error stays the one thrown (hetzner-evidence.ts). An edge on the runner serves the sandbox's agent
 * through a forward added to the tunnel once the sandbox is placed (hetzner-tunnel.ts).
 */
import { describe, expect, test } from 'bun:test'
import { execFileSync } from 'node:child_process'
import { randomBytes } from 'node:crypto'
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { sql } from 'drizzle-orm'
import {
  type ExecEvent,
  ndjson,
  type Sandbox as SandboxView,
  Transport,
} from '../../sdk/typescript/src/transport.ts'
import { smallestShape } from '../src/autoscale.ts'
import { FleetCa } from '../src/ca.ts'
import { loadConfig } from '../src/config.ts'
import { MINUTE_MS } from '../src/durations.ts'
import { createKey } from '../src/keys.ts'
import { startControl } from '../src/main.ts'
import { ProviderClient } from '../src/providers/client.ts'
import type { paths } from '../src/providers/generated/hetzner.ts'
import { HetznerProvider } from '../src/providers/hetzner.ts'
import { one } from '../src/sql.ts'
import { freshDatabase } from '../test/nodes.ts'
import {
  boxEvidence,
  controlEvidence,
  deleteServer,
  deleteWhenFree,
  preflight,
  progress,
  tunnelEvidence,
  wait,
} from './hetzner-evidence.ts'
import { agentEvidence, forwardAgent, startRunnerEdge, tunnelMaster } from './hetzner-tunnel.ts'
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
      let edge: Awaited<ReturnType<typeof startRunnerEdge>> | undefined, sandbox: string | undefined
      let network: number | undefined, firewall: number | undefined, sshKey: number | undefined
      let acquisition: string | undefined, serverId: string | undefined, ip: string | undefined
      let provider: HetznerProvider | undefined,
        failed = false,
        failure: unknown
      const region = 'fsn1' // Test location's network zone is eu-central, per Hetzner's location catalog.
      const tunnelLog = `${dir}/tunnel.log`
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
      try {
        for (const key of ['client', 'host'])
          execFileSync('ssh-keygen', ['-q', '-t', 'ed25519', '-N', '', '-C', name, '-f', `${dir}/${key}`])
        const runnerIp = (await (await fetch('https://api.ipify.org')).text()).trim()
        if (!/^\d+\.\d+\.\d+\.\d+$/.test(runnerIp)) throw new Error('CI public IPv4 unavailable')
        progress('runner address', runnerIp)
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
        progress(
          'made',
          `network ${network}, firewall ${firewall} (tcp 22 from ${runnerIp}), ssh key ${sshKey}`,
        )
        const hostKey = readFileSync(`${dir}/host`, 'utf8'),
          hostPublic = readFileSync(`${dir}/host.pub`, 'utf8').trim()
        provider = new HetznerProvider({
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
            PRYX_EDGE_TOKEN: randomBytes(32).toString('hex'),
            PRYX_PUBLIC_DOMAIN: 'pryx.test',
            PRYX_NODE_LISTEN: '127.0.0.1:8443',
            PRYX_PROVIDER_ALLOW_IMAGES: 'alpine:',
            PRYX_REGION_MIN_NODES: '0',
            PRYX_PRYXD_BIN: process.env.PRYXD_STATIC_BIN,
            PRYX_AGENT_BIN: process.env.PRYX_AGENT_BIN,
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
        const runnerEdge = await startRunnerEdge(current, name)
        edge = runnerEdge
        progress('control started', `node endpoint ${config.joinUrl}`)
        await preflight(current, pems.certPem)
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
        progress('acquisition', `${made.id}, shape ${shape.name} in ${region}`)
        let seen = ''
        const observe = async () => {
          const a = await current.hosts.record(made.id)
          const now = `${a.state}${a.host ? ` host ${a.host.id}` : ''}${a.node_id ? ` node ${a.node_id}` : ''}${a.error ? ` (${a.error})` : ''}`
          if (now !== seen) {
            seen = now
            progress('acquisition state', now)
          }
          return a
        }
        const host = await wait(
          'provider id',
          async () => (await observe()).host,
          10,
          () => seen,
        )
        serverId = host.id
        const server = await api.request<
          paths['/servers/{id}']['get']['responses'][200]['content']['application/json']
        >(`/servers/${serverId}`)
        ip = server.server?.public_net.ipv4?.ip
        if (!ip) throw new Error(`Server ${serverId} has no public IPv4 for SSH`)
        const address = ip
        progress('server', `${serverId} at ${address}, status ${server.server?.status}`)
        writeFileSync(`${dir}/known_hosts`, `${address} ${hostPublic}\n`, { mode: 0o600 })
        let sshSaid = ''
        // StrictHostKeyChecking passes only once user_data has installed the host key, just before the join loop.
        await wait(
          'SSH',
          async () => {
            const p = Bun.spawn([...ssh, `root@${address}`, 'true'], { stdout: 'ignore', stderr: 'pipe' })
            const [code, said] = await Promise.all([p.exited, new Response(p.stderr).text()])
            sshSaid = said.trim().split('\n').at(-1) ?? ''
            return code === 0 ? true : null
          },
          10,
          () => `root@${address} port 22 answered: ${sshSaid || 'nothing'}`,
        )
        tunnel = Bun.spawn(
          [
            ...ssh,
            '-N',
            ...tunnelMaster(dir),
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
            `root@${address}`,
          ],
          { stdout: 'ignore', stderr: Bun.file(tunnelLog) },
        )
        const opened = tunnel
        const rentedFrom = provider
        progress(
          'tunnel opened',
          'box 127.0.0.1:8443 and :9000 to this runner, runner 127.0.0.1:7443 to the box',
        )
        const joined = await wait(
          'first healthy heartbeat',
          async () => {
            if (opened.exitCode !== null)
              throw new Error(`The SSH tunnel exited ${opened.exitCode} before the box joined`)
            const a = await observe()
            return a.ready_at && a.node_id ? a : null
          },
          10,
          () => seen,
        )
        const measurement = await one<{ seconds: number }>(
          current.db,
          sql`SELECT extract(epoch FROM ready_at-requested_at)::float8 AS seconds FROM provider_acquisitions WHERE id=${made.id}`,
        )
        console.info(
          JSON.stringify({ provider: 'hetzner', n: 1, acquireToHealthySeconds: measurement?.seconds }),
        )
        progress('healthy', `node ${joined.node_id} after ${measurement?.seconds} s from request`)
        await current.runtime.refreshRegistry()
        const key = await createKey(current.db, 'default', 'provider-e2e'),
          sdk = new Transport({ url: current.apiUrl, key: key.key })
        const box = await sdk.request<SandboxView>(
          '/v1/sandboxes',
          'POST',
          {
            region,
            image: 'alpine:3.22',
            resources: { memoryMb: 128 },
            entrypoint: ['sh', '-c', 'trap "exit 0" TERM; while :; do sleep 1; done'],
          },
          crypto.randomUUID(),
        )
        progress('sandbox placed', box.id)
        sandbox = box.id
        try {
          await forwardAgent(current, runnerEdge, box.id, ssh, dir, address)
          expect(
            await Array.fromAsync(
              ndjson<ExecEvent>(
                await sdk.raw(
                  `/v1/sandboxes/${encodeURIComponent(box.id)}/exec`,
                  'POST',
                  { command: ['echo', 'provider-live'] },
                  crypto.randomUUID(),
                ),
              ),
            ),
          ).toContainEqual(expect.objectContaining({ type: 'stdout', data: 'provider-live\n' }))
          progress('exec answered', box.id)
        } finally {
          await sdk.request<{ existed: boolean }>(`/v1/sandboxes/${encodeURIComponent(box.id)}`, 'DELETE')
        }
        expect((await cli(['release', joined.node_id ?? ''])).state).toBe('released')
        expect((await rentedFrom.list()).some((h) => h.id === serverId)).toBe(false)
        progress('released', `node ${joined.node_id}, server ${serverId}`)
        serverId = undefined
      } catch (error) {
        failed = true
        failure = error
        console.error('FAILED:', error)
        // Evidence is taken while the box still exists.
        if (ip) boxEvidence(ssh, ip)
        else console.error('BOX had no known address, so SSH was not tried')
        tunnelEvidence(tunnel, tunnelLog)
        if (control && acquisition)
          await controlEvidence(control, acquisition).catch((e) =>
            console.error('CONTROL evidence unavailable:', e),
          )
        if (control && sandbox)
          await agentEvidence(control, sandbox).catch((e) =>
            console.error('CONTROL agent route unavailable:', e),
          )
        console.error('CONTROL logs to this process, so its lines, if any, are above')
      }
      // Only test-created resources are removed. A failed bootstrap must not leak a paid server.
      const cleanup: unknown[] = []
      const step = async (what: string, run: () => Promise<unknown>) => {
        try {
          await run()
        } catch (error) {
          cleanup.push(error)
          console.error(`CLEANUP ${what} failed:`, error)
        }
      }
      await step('finding the server', async () => {
        if (control && acquisition) serverId ??= (await control.hosts.record(acquisition)).host?.id
        // A create whose answer was lost leaves no host on the record, only the labelled server.
        if (provider && acquisition && !serverId)
          serverId = (await provider.list()).find((h) => h.requestKey === acquisition)?.id
      })
      const rented = serverId
      if (rented) await step(`deleting server ${rented} (delete it by hand)`, () => deleteServer(api, rented))
      await step('closing the tunnel', async () => {
        tunnel?.kill()
        await tunnel?.exited
      })
      await step('stopping the edge', async () => edge?.stop())
      await step('stopping control', async () => control?.stop())
      for (const [path, id] of [
        ['firewalls', firewall],
        ['networks', network],
        ['ssh_keys', sshKey],
      ] as const)
        if (id) await deleteWhenFree(api, path, id)
      await step('dropping the database', () => fresh.drop())
      rmSync(dir, { recursive: true, force: true })
      const said = (e: unknown) => (e instanceof Error ? e.message : String(e))
      if (failed && cleanup.length)
        throw new AggregateError(
          [failure, ...cleanup],
          `${said(failure)} (cleanup also failed: ${cleanup.map(said).join('; ')})`,
        )
      if (failed) throw failure
      if (cleanup.length) throw new AggregateError(cleanup, `Cleanup failed: ${cleanup.map(said).join('; ')}`)
    },
    20 * MINUTE_MS,
  )
})
