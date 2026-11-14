// SPDX-License-Identifier: FSL-1.1-ALv2
/** Welcome secrecy and authorization against real Postgres keys, including revocation. */

import { describe, expect, test } from 'bun:test'
import type { AddressInfo } from 'node:net'
import { sql } from 'drizzle-orm'
import { Hono } from 'hono'
import { StatusCodes } from 'http-status-codes'
import { edgeConfig } from '../../edge/src/config.ts'
import { front } from '../../edge/src/front.ts'
import { Hold } from '../../edge/src/hold.ts'
import { Gate } from '../../edge/src/limits.ts'
import { Metrics } from '../../edge/src/metrics.ts'
import { RouteTable } from '../../edge/src/routes.ts'
import { SandboxProxy } from '../../edge/src/sandbox-proxy.ts'
import { HTTP_FRONT_PORT } from '../src/edge-headers.ts'
import { bootstrapKey } from '../src/keys.ts'
import { withWelcome } from '../src/welcome.ts'
import { WELCOME_DETAILS_PATH } from '../src/welcome-path.ts'
import { freshDatabase, hasDatabase } from './nodes.ts'

describe.skipIf(!hasDatabase)('install welcome', () => {
  test('public HTML hides keys and host details; private examples require an active key', async () => {
    const fresh = await freshDatabase()
    const key = 'platform-generated-welcome-test-key'
    try {
      await bootstrapKey(fresh.db, 'welcome', key)
      const app = new Hono()
      withWelcome(app, {
        db: fresh.db,
        origin: 'https://install.example.test',
        isolation: 'process',
        platform: 'render',
        platformKey: true,
        boundary: 'shared UID',
      })
      const page = await app.request('/')
      const html = await page.text()
      expect(page.headers.get('cache-control')).toBe('no-store')
      expect(page.headers.get('content-security-policy')).toContain("frame-ancestors 'none'")
      expect(html).toContain('Render → service → Environment')
      for (const hidden of [
        key,
        'process',
        'Landlock',
        'install.example.test',
        'sessionStorage',
        'localStorage',
      ])
        expect(html).not.toContain(hidden)
      for (const authorization of ['', 'Bearer wrong']) {
        const denied = await app.request(WELCOME_DETAILS_PATH, { headers: { authorization } })
        expect(denied.status).toBe(StatusCodes.UNAUTHORIZED)
        expect(await denied.text()).not.toContain('examples')
      }
      const unlocked = await app.request(WELCOME_DETAILS_PATH, {
        headers: { authorization: `Bearer ${key}` },
      })
      const body = await unlocked.text()
      expect(unlocked.status).toBe(StatusCodes.OK)
      expect(unlocked.headers.get('cache-control')).toBe('no-store')
      expect(body).toContain('Default tier: process')
      expect(body).toContain('Boundary: shared UID')
      expect(body).toContain('mutually trusted')
      expect(body).toContain('https://install.example.test')
      expect(body).not.toContain(key)
      expect(JSON.parse(body).examples).toHaveLength(6)
      await fresh.db.execute(sql`UPDATE keys SET revoked_at=now()`)
      expect(
        (await app.request(WELCOME_DETAILS_PATH, { headers: { authorization: `Bearer ${key}` } })).status,
      ).toBe(StatusCodes.UNAUTHORIZED)
    } finally {
      await fresh.drop()
    }
  })
})

describe.skipIf(!hasDatabase)('welcome through the public image gateway', () => {
  test('the public root and private details reach control through the agent proxy', async () => {
    const fresh = await freshDatabase()
    const key = 'gateway-welcome-key'
    await bootstrapKey(fresh.db, 'welcome-gateway', key)
    const app = new Hono()
    withWelcome(app, { db: fresh.db, isolation: 'process' })
    const upstream = Bun.serve({ hostname: '127.0.0.1', port: 0, fetch: app.fetch })
    const table = new RouteTable()
    const config = edgeConfig({ PRYX_EDGE_TOKEN: 'gateway-welcome-sufficient-length-test-token' })
    const hold = new Hold(async () => {
      throw new Error('Welcome must not wake a sandbox')
    })
    const proxy = new SandboxProxy(table, hold, config.holdMs)
    proxy.useControl(upstream.url.toString())
    const listener = await front({
      host: '127.0.0.1',
      port: 0,
      kind: 'http',
      routePort: HTTP_FRONT_PORT,
      table,
      agents: proxy,
      gate: new Gate(config.limits),
      hold,
      holdMs: config.holdMs,
      greetingMs: config.greetingMs,
      bufferBytes: config.bufferBytes,
      metrics: new Metrics(),
      session: () => {},
      control: { destination: proxy.destination() as string, paths: /^\//, hostname: '127.0.0.1' },
    })
    const origin = `http://127.0.0.1:${(listener.address() as AddressInfo).port}`
    try {
      const page = await fetch(origin)
      expect(page.status).toBe(StatusCodes.OK)
      expect(await page.text()).toContain('pryx runs here')
      expect((await fetch(origin + WELCOME_DETAILS_PATH)).status).toBe(StatusCodes.UNAUTHORIZED)
      const details = await fetch(origin + WELCOME_DETAILS_PATH, {
        headers: { authorization: `Bearer ${key}` },
      })
      expect(details.status).toBe(StatusCodes.OK)
      expect((await details.json()).examples).toHaveLength(6)
    } finally {
      listener.closeClients()
      await new Promise<void>((resolve) => listener.close(() => resolve()))
      proxy.close()
      upstream.stop(true)
      await fresh.drop()
    }
  })
})
