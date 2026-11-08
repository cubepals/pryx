// SPDX-License-Identifier: FSL-1.1-ALv2
/** Sealing binds credentials to sandbox/project identity and storage never contains plaintext. */
import { afterAll, beforeAll, expect, test } from 'bun:test'
import { sql } from 'drizzle-orm'
import { type MountSpec, NativeMounts } from '../src/native-mounts.ts'
import { one } from '../src/sql.ts'
import { freshDatabase } from './nodes.ts'

let database: Awaited<ReturnType<typeof freshDatabase>>
beforeAll(async () => {
  database = await freshDatabase()
})
afterAll(async () => {
  await database.drop()
})
test('AES-GCM credentials are randomized, authenticated and scoped', async () => {
  const store = new NativeMounts(database.db, 'test-seal-key-with-at-least-thirty-two-bytes')
  const specs: MountSpec[] = [
    {
      bucket: 's3://fixture/prefix',
      path: '/mnt/data',
      readOnly: false,
      credentials: {
        accessKeyId: 'access-private',
        secretAccessKey: 'secret-private',
        sessionToken: null,
        endpoint: 'http://storage.invalid:9000',
        region: 'test',
      },
    },
  ]
  const first = store.seal('one', 'project', specs),
    second = store.seal('one', 'project', specs)
  expect(first).not.toBe(second)
  expect(store.open('one', 'project', first)).toEqual(specs)
  expect(() => store.open('two', 'project', first)).toThrow()
  expect(() => store.open('one', 'other', first)).toThrow()
  const broken = Buffer.from(first, 'base64')
  broken[broken.length - 1] = (broken[broken.length - 1] ?? 0) ^ 1
  expect(() => store.open('one', 'project', broken.toString('base64'))).toThrow()
  await store.save('one', 'project', specs)
  const row = await one<{ sealed: string }>(
    database.db,
    sql`SELECT sealed FROM sandbox_mounts WHERE sandbox='one'`,
  )
  expect(row?.sealed).not.toContain('secret-private')
  expect(row?.sealed).not.toContain('access-private')
  await store.remove('one')
  expect(await one(database.db, sql`SELECT 1 FROM sandbox_mounts WHERE sandbox='one'`)).toBeNull()
})

test('SSH idle expiry only tightens and cannot revive after a wake', async () => {
  const { ensureProject } = await import('../src/project-admission.ts')
  const { randomUUID } = await import('node:crypto')
  const { SECOND_MS } = await import('../src/durations.ts')
  const id = 'ssh-idle-case',
    lease = randomUUID()
  await ensureProject(database.db, 'project')
  await database.db.execute(
    sql`INSERT INTO sandboxes(id,project,spec,state,region) VALUES(${id},'project','{"idle":2}'::jsonb,'running','default')`,
  )
  await database.db.execute(
    sql`INSERT INTO sandbox_ssh(id,sandbox,project,epoch,slot,target_port,username,host_key,expires_at) VALUES(${lease},${id},'project',1,22000,2222,'pryx','public',clock_timestamp()+interval '5 minutes')`,
  )
  await database.db.execute(sql`UPDATE sandboxes SET state='sleeping' WHERE id=${id}`)
  const first = await one<{ expires_at: string }>(
    database.db,
    sql`SELECT expires_at FROM sandbox_ssh WHERE id=${lease}`,
  )
  expect(Date.parse(String(first?.expires_at)) - Date.now()).toBeLessThanOrEqual(2 * SECOND_MS)
  await database.db.execute(
    sql`UPDATE sandbox_ssh SET sleep_started_at=clock_timestamp()-interval '1 second' WHERE id=${lease}`,
  )
  await database.db.execute(sql`UPDATE sandboxes SET spec='{"idle":0.1}'::jsonb WHERE id=${id}`)
  const shorter = await one<{ expires_at: string }>(
    database.db,
    sql`SELECT expires_at FROM sandbox_ssh WHERE id=${lease}`,
  )
  expect(Date.parse(String(shorter?.expires_at))).toBeLessThan(Date.now())
  await database.db.execute(sql`UPDATE sandboxes SET state='running' WHERE id=${id}`)
  const woke = await one<{ expires_at: string; sleep_started_at: string | null }>(
    database.db,
    sql`SELECT expires_at,sleep_started_at FROM sandbox_ssh WHERE id=${lease}`,
  )
  expect(woke?.sleep_started_at).toBeNull()
  expect(String(woke?.expires_at)).toBe(String(shorter?.expires_at))
})
