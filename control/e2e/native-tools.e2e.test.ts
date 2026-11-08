// SPDX-License-Identifier: FSL-1.1-ALv2
/** Real native services cross both SDKs, scoped agent routes and the edge. Every readiness
 * sample includes create-to-first-image, and failures clean each created sandbox. */
import { expect, test } from 'bun:test'
import { writeFileSync } from 'node:fs'
import { GetObjectCommand, PutObjectCommand, S3Client } from '@aws-sdk/client-s3'
import { StatusCodes } from 'http-status-codes'
import { builtinTemplate } from '../../cli/src/builtin-template.ts'
import { connectSsh } from '../../cli/src/sandbox-ssh.ts'
import { TOOL_DEFAULTS } from '../../sdk/typescript/src/generated/tools.ts'
import type { Sandbox } from '../../sdk/typescript/src/index.ts'
import { MINUTE_MS, SECOND_MS } from '../src/durations.ts'
import { agentDeployment } from './agent-context.ts'
import { ready } from './deployment.ts'
import { deploymentDiagnostics } from './diagnostics.ts'
import { until } from './waits.ts'

const PNG = [137, 80, 78, 71, 13, 10, 26, 10]
function png(bytes: Uint8Array) {
  expect([...bytes.subarray(0, 8)]).toEqual(PNG)
  expect(bytes.length).toBeGreaterThan(100)
}
async function command(argv: string[], env?: Record<string, string | undefined>) {
  const child = Bun.spawn(argv, { env, stdout: 'pipe', stderr: 'pipe' })
  const [code, out, error] = await Promise.all([
    child.exited,
    new Response(child.stdout).text(),
    new Response(child.stderr).text(),
  ])
  if (code !== 0) throw new Error(`Native tool check exited ${code}: ${error.slice(-2000)}`)
  return out
}
async function python(env: Record<string, string | undefined>, box: Sandbox, kind: string) {
  await command([process.env.PRYX_PYTHON ?? 'python3', 'sdk/python/e2e/native_tools.py'], {
    ...env,
    PRYX_TOOLS_SANDBOX: box.id,
    PRYX_TOOLS_KIND: kind,
  })
}
test(
  'desktop, SSH wake, real S3 FUSE and browser work through both clients',
  async () => {
    expect(ready).toBe(true)
    const c = await agentDeployment('container', { nativeTools: true })
    let box: Sandbox | undefined
    const samples: Record<string, number[]> = {}
    try {
      for (const [name, image] of [
        ['desktop', 'alpine:pryx-desktop'],
        ['browser', 'alpine:pryx-browser'],
        ['tools', 'alpine:pryx-native-tools'],
      ]) {
        const plan = builtinTemplate(name as string, image)
        if (!plan) throw new Error('Missing native plan')
        await c.sdk.template(plan)
      }
      box = await c.sdk.sandbox({ template: 'desktop', units: 2, idle: '10m' })
      await c.route(box.id)
      png(await box.desktop.screenshot())
      const desktopBox = box
      await desktopBox.spawn(['urxvt', '-fn', 'xft:DejaVu Sans Mono:size=10', '-title', 'pryx-desktop-check'])
      const window = await until('desktop terminal appears', async () => {
        const result = await desktopBox.run(['xdotool', 'search', '--name', 'pryx-desktop-check'])
        return result.code === 0 ? result.stdout.trim().split(/\s+/)[0] : undefined
      })
      expect(
        (
          await desktopBox.run([
            'xdotool',
            'windowmove',
            window,
            '100',
            '100',
            'windowsize',
            window,
            '640',
            '480',
            'windowactivate',
            '--sync',
            window,
          ])
        ).code,
      ).toBe(0)
      await box.desktop.click(150, 150)
      await box.desktop.type('printf desktop-input-ok > /data/desktop-input')
      await box.desktop.key('Return')
      await until(
        'desktop input reaches its shell',
        async () => (await desktopBox.read('desktop-input')) === 'desktop-input-ok',
      )
      const desktop = await box.desktop(),
        page = new URL(desktop.url)
      expect((await fetch(page)).status).toBe(StatusCodes.OK)
      const wsPath = new URLSearchParams(page.hash.slice(1)).get('path')
      expect(wsPath).toContain('token=')
      if (!wsPath) throw new Error('Missing noVNC WebSocket path')
      const vnc = new URL(wsPath, c.origin)
      vnc.protocol = 'ws:'
      const banner = await new Promise<string>((resolve, reject) => {
        const ws = new WebSocket(vnc),
          timer = setTimeout(() => {
            ws.close()
            reject(new Error('VNC banner timeout'))
          }, 5 * SECOND_MS)
        ws.onerror = () => {
          clearTimeout(timer)
          reject(new Error('noVNC WebSocket refused'))
        }
        ws.onmessage = async (event) => {
          clearTimeout(timer)
          const bytes = event.data instanceof Blob ? await event.data.arrayBuffer() : event.data
          ws.close()
          resolve(typeof bytes === 'string' ? bytes : new TextDecoder().decode(bytes))
        }
      })
      expect(banner).toStartWith('RFB ')

      const without = await fetch(
        new URL(`/sandboxes/${box.id}/ports/${TOOL_DEFAULTS.desktopPort}/websockify`, c.origin),
        { headers: { upgrade: 'websocket' } },
      )
      expect(without.ok).toBe(false)
      const cap = await c.d.control.edge.agents?.token(box.id, 'default', c.d.control.apiUrl)
      if (!cap) throw new Error('Missing scoped capability')
      const mcp = (await fetch(`${cap.url}/_pryx/mcp`, {
        method: 'POST',
        headers: { authorization: `Bearer ${cap.token}`, 'content-type': 'application/json' },
        body: JSON.stringify({
          jsonrpc: '2.0',
          id: 1,
          method: 'tools/call',
          params: { name: 'screenshot', arguments: {} },
        }),
      }).then((r) => r.json())) as { result: { content: { type: string; mimeType?: string }[] } }
      expect(mcp.result.content.some((v) => v.type === 'image' && v.mimeType === 'image/png')).toBe(true)
      await python(c.d.env, box, 'desktop')
      await box.delete()
      box = undefined
      box = await c.sdk.sandbox({ template: 'tools', idle: '2m' })
      const lease = await box.ssh()
      expect(JSON.stringify(lease)).not.toContain('PRIVATE KEY')
      const routeId = `${box.id}:ssh-${lease.id}`
      await until('edge holds raw SSH lease', async () => c.edge.table.held.get(routeId))
      await box.sleep()
      await until('edge sees SSH asleep', async () => c.edge.table.held.get(routeId)?.state)
      expect(await connectSsh(lease, ['printf ssh-raw-woke'])).toBe(0)
      await until('raw SSH woke the sandbox', async () => (await box?.refresh())?.state === 'running')
      // The compiled CLI separately exercises acquisition, pinned-host login and revoke.
      await command(
        [
          process.env.PRYX_MCP_CLI ?? process.execPath,
          ...(process.env.PRYX_MCP_CLI ? [] : ['cli/src/main.ts']),
          'sandbox',
          'ssh',
          box.id,
          'printf ssh-ok',
        ],
        c.d.env,
      )
      await until('SSH woke the sandbox', async () => (await box?.refresh())?.state === 'running')
      await python(c.d.env, box, 'ssh')
      await box.ssh.revoke()
      await until('SSH revoke removes the raw route', async () => !c.edge.table.held.has(routeId))
      const container = `blockly-${c.d.control.config.deployment}-${box.id}`
      const info = JSON.parse(await command(['docker', 'inspect', container])) as {
        NetworkSettings: { Networks: Record<string, { Gateway: string }> }
      }[]
      const gateway = Object.values(info[0]?.NetworkSettings.Networks ?? {})[0]?.Gateway
      if (!gateway) throw new Error('No sandbox gateway for S3 fixture')
      await box.delete()
      box = undefined
      const credentials = {
        accessKeyId: process.env.S3_TEST_ACCESS_KEY_ID ?? 'blockly-test',
        secretAccessKey: process.env.S3_TEST_SECRET_ACCESS_KEY ?? 'blockly-test-secret',
      }
      const bucket = c.d.control.config.s3.bucket,
        s3 = new S3Client({
          endpoint: process.env.S3_TEST_ENDPOINT,
          region: 'us-east-1',
          forcePathStyle: true,
          credentials,
        })
      await s3.send(new PutObjectCommand({ Bucket: bucket, Key: 'native/seed', Body: 'seed' }))
      box = await c.sdk.sandbox({
        image: 'alpine:pryx-native-tools',
        idle: '10m',
        mounts: [
          {
            bucket: `s3://${bucket}/native`,
            path: '/mnt/data',
            readOnly: false,
            credentials: { ...credentials, endpoint: `http://${gateway}:9000`, region: 'us-east-1' },
          },
        ],
      })
      expect((await box.mounts())[0]?.state).toBe('mounted')
      expect(
        (await box.run(['sh', '-c', 'cat /mnt/data/seed && printf ts > /mnt/data/ts-object'])).stdout,
      ).toBe('seed')
      expect(
        await (
          await s3.send(new GetObjectCommand({ Bucket: bucket, Key: 'native/ts-object' }))
        ).Body?.transformToString(),
      ).toBe('ts')
      expect((await box.run(['sh', '-c', "grep '^CapEff:' /proc/self/status"])).stdout).toMatch(/0{16}/)
      await python(c.d.env, box, 'mount')
      await box.delete()
      box = undefined
      box = await c.sdk.sandbox({
        image: 'alpine:pryx-rclone-tools',
        idle: '10m',
        mounts: [
          {
            bucket: `s3://${bucket}/native`,
            path: '/mnt/data',
            readOnly: false,
            credentials: { ...credentials, endpoint: `http://${gateway}:9000`, region: 'us-east-1' },
          },
        ],
      })
      expect((await box.mounts())[0]?.backend).toBe('rclone')
      expect(
        (await box.run(['sh', '-c', 'cat /mnt/data/seed && printf rclone > /mnt/data/rclone-object'])).stdout,
      ).toBe('seed')
      await until('rclone flushed its object', async () => {
        try {
          return (
            (await (
              await s3.send(new GetObjectCommand({ Bucket: bucket, Key: 'native/rclone-object' }))
            ).Body?.transformToString()) === 'rclone'
          )
        } catch {
          return false
        }
      })
      await python(c.d.env, box, 'mount')
      await box.delete()
      box = undefined

      box = await c.sdk.sandbox({
        image: 'alpine:pryx-native-tools',
        mounts: [
          {
            bucket: `s3://${bucket}/native`,
            path: '/mnt/data',
            readOnly: true,
            credentials: { ...credentials, endpoint: `http://${gateway}:9000`, region: 'us-east-1' },
          },
        ],
      })
      expect((await box.run(['sh', '-c', 'printf denied > /mnt/data/denied'])).code).not.toBe(0)
      await box.delete()
      box = undefined
      for (const kind of ['desktop', 'browser'] as const) {
        for (let i = 0; i < 30; i++) {
          let measured = 0
          const started = performance.now()
          box = await c.sdk.sandbox({ template: kind, units: 4, idle: '10m' })
          if (kind === 'browser') {
            await box.write('index.html', '<!doctype html><title>native</title><h1>Rendered page</h1>')
            const server = await box.spawn([
              'python3',
              '-m',
              'http.server',
              '3000',
              '--bind',
              '127.0.0.1',
              '--directory',
              '/data',
            ])
            await server.waitForPort(3000)
            png(await box.browser.screenshot('http://127.0.0.1:3000'))
            measured = performance.now() - started
            if (i === 0) {
              const browser = await box.browser()
              const refused = await fetch(browser.url.replace(/\?.*$/, ''))
              expect(refused.status).toBe(StatusCodes.FORBIDDEN)
              await command(['node', 'templates/browser/cdp-check.cjs', browser.url])
              await python(c.d.env, box, 'browser')
            }
          } else {
            png(await box.desktop.screenshot())
            measured = performance.now() - started
          }
          samples[`${kind}_create_to_first_image`] ??= []
          samples[`${kind}_create_to_first_image`]?.push(measured)
          await box.delete()
          box = undefined
        }
      }
      const summary = Object.fromEntries(
        Object.entries(samples).map(([name, values]) => {
          const sorted = [...values].sort((a, b) => a - b)
          return [
            name,
            {
              n: values.length,
              p50Ms: sorted[Math.ceil(values.length * 0.5) - 1],
              p90Ms: sorted[Math.ceil(values.length * 0.9) - 1],
            },
          ]
        }),
      )
      const output = {
        before: {
          commit: '8bfb636',
          desktop: 'agent route unavailable before K18',
          browser: 'agent route unavailable before K18',
        },
        after: { samples, summary },
        resources: { units: 4 },
        scope: 'create on a warm template through first PNG; browser includes local page server startup',
      }
      writeFileSync(
        process.env.PRYX_NATIVE_MEASUREMENTS ?? '/tmp/pryx-native-measurements.json',
        `${JSON.stringify(output, null, 2)}\n`,
      )
    } catch (error) {
      deploymentDiagnostics(c.d)
      throw error
    } finally {
      await box?.delete()
      await c.close()
    }
  },
  30 * MINUTE_MS,
)
