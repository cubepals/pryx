// SPDX-License-Identifier: FSL-1.1-ALv2
/** Welcome secrecy and authorization against real Postgres keys, including revocation. */
import { describe, expect, test } from 'bun:test'
import { sql } from 'drizzle-orm'
import { Hono } from 'hono'
import { StatusCodes } from 'http-status-codes'
import { bootstrapKey } from '../src/keys.ts'
import { withWelcome } from '../src/welcome.ts'
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
        const denied = await app.request('/_pryx/welcome', { headers: { authorization } })
        expect(denied.status).toBe(StatusCodes.UNAUTHORIZED)
        expect(await denied.text()).not.toContain('examples')
      }
      const unlocked = await app.request('/_pryx/welcome', { headers: { authorization: `Bearer ${key}` } })
      const body = await unlocked.text()
      expect(unlocked.status).toBe(StatusCodes.OK)
      expect(unlocked.headers.get('cache-control')).toBe('no-store')
      expect(body).toContain('Default tier: process')
      expect(body).toContain('https://install.example.test')
      expect(body).not.toContain(key)
      expect(JSON.parse(body).examples).toHaveLength(6)
      await fresh.db.execute(sql`UPDATE keys SET revoked_at=now()`)
      expect(
        (await app.request('/_pryx/welcome', { headers: { authorization: `Bearer ${key}` } })).status,
      ).toBe(StatusCodes.UNAUTHORIZED)
    } finally {
      await fresh.drop()
    }
  })
})
