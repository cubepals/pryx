// SPDX-License-Identifier: FSL-1.1-ALv2
/** Project isolation, admission and lifecycle against real Postgres. Node behavior uses the established runtime double. */
import { describe, expect, test } from 'bun:test'
import { sql } from 'drizzle-orm'
import { StatusCodes } from 'http-status-codes'
import { CodeTemplates } from '../src/code-templates.ts'
import { THRESHOLDS } from '../src/health.ts'
import { createKey } from '../src/keys.ts'
import { createOperatorApi } from '../src/operator-api.ts'
import { type ProjectAdmission, projectUsage } from '../src/project-admission.ts'
import { withProjects } from '../src/project-api.ts'
import { freeUnits, LIMIT_ENV, type ProjectLimits, projectDefaults } from '../src/project-model.ts'
import { Projects } from '../src/projects.ts'
import { rows } from '../src/sql.ts'
import { publicContext } from './public-context.ts'

describe('projects', () => {
  for (const source of ['project', 'snapshot', 'template'] as const)
    test(`auto-delete uses the effective idle duration from ${source}`, async () => {
      const c = await publicContext(),
        projects = new Projects(c.boxes.projects as ProjectAdmission, c.boxes)
      try {
        let body: object
        if (source === 'project') {
          await projects.set('default', { maxIdle: 30 })
          body = { image: 'alpine:3.22', autoDelete: true }
        } else {
          const box = (await (
            await c.request('/sandboxes', 'POST', { image: 'alpine:3.22', idle: 30 })
          ).json()) as { id: string }
          const snapshot = (await (await c.request(`/sandboxes/${box.id}/snapshots`, 'POST')).json()) as {
            id: string
          }
          if (source === 'template')
            await new CodeTemplates(c.db, c.boxes).register('default', 'idle-template', {
              snapshot: snapshot.id,
              plan: {},
            })
          body = { [source]: source === 'template' ? 'idle-template' : snapshot.id, autoDelete: true }
        }
        const response = await c.request('/sandboxes', 'POST', body)
        expect(response.status).toBe(StatusCodes.CREATED)
        const box = (await response.json()) as { id: string }
        expect((await c.boxes.record(box.id)).spec).toMatchObject({ autoDelete: true, idle: 30 })
        await projects.set('default', { maxIdle: 10 })
        const refused = await c.request('/sandboxes', 'POST', { ...body, idle: 30 })
        expect(refused.status).toBe(StatusCodes.FORBIDDEN)
      } finally {
        await c.close()
      }
    })
  for (const action of ['freeze', 'remove', 'upkeep', 'ttl'] as const)
    test(`${action} handles expired template builders without losing retained snapshots`, async () => {
      const c = await publicContext(),
        projects = new Projects(c.boxes.projects as ProjectAdmission, c.boxes),
        templates = new CodeTemplates(c.db, c.boxes)
      try {
        await projects.set('default', { maxTtl: 60 })
        const response = await c.request('/sandboxes', 'POST', { image: 'alpine:3.22' })
        expect(response.status).toBe(StatusCodes.CREATED)
        const box = (await response.json()) as { id: string }
        expect((await c.boxes.record(box.id)).spec.ttl).toBe(60)
        const capture = await c.request(`/sandboxes/${box.id}/snapshots`, 'POST')
        expect(capture.status).toBe(StatusCodes.CREATED)
        const snapshot = (await capture.json()) as { id: string }
        await templates.register('default', 'retained', { snapshot: snapshot.id, plan: {} })
        await c.db.execute(sql`UPDATE sandboxes SET expires_at='2000-01-01T00:00:00Z' WHERE id=${box.id}`)
        await expect(c.boxes.stop(box.id, 'default')).rejects.toMatchObject({ code: 'gone' })
        await expect(c.boxes.stop(box.id, 'default', true)).rejects.toMatchObject({ code: 'gone' })
        await expect(c.boxes.delete(box.id)).rejects.toMatchObject({ code: 'forbidden' })
        if (action === 'upkeep') {
          await projects.set('default', { expiresAt: '2000-01-01T00:00:00Z' })
          await projects.upkeep()
        } else if (action === 'ttl') await c.boxes.upkeep()
        else await projects[action]('default')
        const state = await rows<{ state: string; deleted_at: string | null; quota_running: boolean }>(
          c.db,
          sql`SELECT state,deleted_at,quota_running FROM sandboxes WHERE id=${box.id}`,
        )
        expect(state[0]?.state).toBe('stopped')
        expect(state[0]?.quota_running).toBe(false)
        if (action === 'freeze' || action === 'ttl') {
          expect(state[0]?.deleted_at).toBeNull()
          expect((await templates.versions('default', 'retained')).length).toBe(1)
          expect(
            await rows(c.db, sql`SELECT deleted_at FROM sandbox_snapshots WHERE id=${snapshot.id}`),
          ).toEqual([{ deleted_at: null }])
          expect((await projects.get('default')).state).toBe(action === 'freeze' ? 'frozen' : 'active')
          await expect(c.boxes.stop(box.id, 'default')).rejects.toMatchObject({ code: 'gone' })
          if (action === 'ttl') {
            await c.boxes.upkeep()
            expect((await templates.versions('default', 'retained')).length).toBe(1)
            await expect(c.boxes.delete(box.id)).rejects.toMatchObject({ code: 'forbidden' })
          }
        } else {
          expect(state[0]?.deleted_at).not.toBeNull()
          expect(await templates.versions('default', 'retained')).toEqual([])
          expect(await rows(c.db, sql`SELECT id FROM sandbox_snapshots WHERE deleted_at IS NULL`)).toEqual([])
          expect(await rows(c.db, sql`SELECT state FROM projects WHERE id='default'`)).toEqual([
            { state: 'deleted' },
          ])
        }
      } finally {
        await c.close()
      }
    })
  test('units round both resources, timer defaults are bounded, and owned delete remains idempotent', async () => {
    const c = await publicContext(),
      projects = new Projects(c.boxes.projects as ProjectAdmission, c.boxes)
    try {
      await projects.set('default', { maxTtl: 60, maxIdle: 30 })
      for (const body of [{ units: 2 }, { resources: { memoryMb: 513, cpuMillis: 300 } }]) {
        const response = await c.request('/sandboxes', 'POST', { image: 'alpine:3.22', ...body })
        expect(response.status).toBe(StatusCodes.CREATED)
        const box = (await response.json()) as { id: string; units: number }
        expect(box.units).toBe(2)
        const row = await c.boxes.record(box.id, 'default')
        expect(row.spec.resources).toMatchObject({ memoryMb: 1024, cpuMillis: 500 })
        expect(row.spec).toMatchObject({ ttl: 60, idle: 30 })
        expect((await c.request(`/sandboxes/${box.id}`, 'DELETE')).status).toBe(StatusCodes.OK)
        expect(await (await c.request(`/sandboxes/${box.id}`, 'DELETE')).json()).toEqual({ existed: false })
      }
      expect(
        (
          await c.request('/sandboxes', 'POST', {
            image: 'alpine:3.22',
            units: 1,
            resources: { memoryMb: 512 },
          })
        ).status,
      ).toBe(StatusCodes.UNPROCESSABLE_ENTITY)
    } finally {
      await c.close()
    }
  })
  test('a snapshot deleted while uploading cannot return as ready', async () => {
    const c = await publicContext()
    let release: (() => void) | undefined
    try {
      const box = (await (await c.request('/sandboxes', 'POST', { image: 'alpine:3.22' })).json()) as {
        id: string
      }
      let entered!: () => void
      const began = new Promise<void>((resolve) => {
        entered = resolve
      })
      const gate = new Promise<void>((resolve) => {
        release = resolve
      })
      c.nodes.once(/^POST .*\/snapshots\/[^/]+\/upload$/, async (answer) => {
        entered()
        await gate
        return answer()
      })
      const capture = c.request(`/sandboxes/${box.id}/snapshots`, 'POST')
      await began
      const snap = (await (await capture).json()) as { id: string }
      expect((await c.request(`/sandboxes/${box.id}/snapshots/${snap.id}`, 'DELETE')).status).toBe(
        StatusCodes.OK,
      )
      release?.()
      await expect(c.boxes.engine.upload(snap.id, true)).rejects.toThrow('has not reached the archive store')
      expect(await rows(c.db, sql`SELECT status,object_key FROM fleet_archives WHERE id=${snap.id}`)).toEqual(
        [{ status: 'deleted', object_key: null }],
      )
      await c.request(`/sandboxes/${box.id}`, 'DELETE')
    } finally {
      release?.()
      await c.close()
    }
  })
  test('every sandbox endpoint, fork and template lookup hides another project', async () => {
    const c = await publicContext()
    try {
      const box = (await (await c.request('/sandboxes', 'POST', { image: 'alpine:3.22' })).json()) as {
        id: string
      }
      const snap = (await (await c.request(`/sandboxes/${box.id}/snapshots`, 'POST')).json()) as {
        id: string
      }
      const template = (await (
        await c.request('/templates', 'POST', { name: 'private-template', spec: { image: 'alpine:3.22' } })
      ).json()) as { id: string }
      const other = await createKey(c.db, 'other', 'other')
      const request = (path: string, method = 'GET', body?: unknown) =>
        c.app.request(`http://control.test/v1${path}`, {
          method,
          headers: { authorization: `Bearer ${other.key}`, 'content-type': 'application/json' },
          ...(body ? { body: JSON.stringify(body) } : {}),
        })
      const calls = c.nodes.calls.length
      for (const [suffix, method, body] of [
        ['', 'GET'],
        ['', 'DELETE'],
        ['/start', 'POST'],
        ['/stop', 'POST'],
        ['/sleep', 'POST'],
        ['/wake', 'POST'],
        ['/rest', 'POST'],
        ['/exec', 'POST', { command: ['true'] }],
        ['/logs', 'GET'],
        ['/files/file', 'GET'],
        ['/files/file', 'PUT', { dataBase64: 'eA==' }],
        ['/file-chunks/file', 'GET'],
        ['/file-chunks/file', 'PUT', { offset: 0, dataBase64: 'eA==' }],
        ['/snapshots', 'GET'],
        ['/snapshots', 'POST'],
        [`/snapshots/${snap.id}`, 'DELETE'],
        ['/events', 'GET'],
        ['/endpoints', 'GET'],
      ] as const)
        expect((await request(`/sandboxes/${box.id}${suffix}`, method, body)).status).toBe(
          StatusCodes.NOT_FOUND,
        )
      expect((await request(`/snapshots/${snap.id}/sandboxes`, 'POST', {})).status).toBe(
        StatusCodes.NOT_FOUND,
      )
      for (const value of [template.id, 'private-template'])
        expect((await request('/sandboxes', 'POST', { template: value })).status).toBe(StatusCodes.NOT_FOUND)
      expect(await (await request('/sandboxes')).json()).toEqual({ sandboxes: [] })
      expect(await (await request('/templates')).json()).toEqual({ templates: [] })
      expect(c.nodes.calls.length).toBe(calls)
      const projects = new Projects(c.boxes.projects as ProjectAdmission, c.boxes)
      await projects.freeze('other')
      expect((await request(`/sandboxes/${box.id}/exec`, 'POST', { command: ['true'] })).status).toBe(
        StatusCodes.NOT_FOUND,
      )
      await c.request(`/sandboxes/${box.id}`, 'DELETE')
    } finally {
      await c.close()
    }
  })
  test('each creation limit returns its name, limit and current count', async () => {
    const c = await publicContext()
    const projects = new Projects(c.boxes.projects as ProjectAdmission, c.boxes)
    try {
      for (const [name, limit, extra] of [
        ['maxUnits', 0, {}],
        ['maxSandboxes', 0, {}],
        ['maxRunning', 0, {}],
        ['maxUnitsPerSandbox', 0, {}],
        ['maxTtl', 4, { ttl: 5 }],
        ['maxTtl', 0, {}],
        ['maxIdle', 4, { idle: 5 }],
        ['maxEgressPorts', 0, { ports: [{ name: 'web', port: 3000, audience: 'public' }] }],
        ['allowedImages', ['node:'], {}],
        ['expiresAt', '2000-01-01T00:00:00.000Z', {}],
      ] as const) {
        await projects.set('default', { [name]: limit })
        const response = await c.request('/sandboxes', 'POST', { image: 'alpine:3.22', ...extra })
        expect(response.status).toBe(StatusCodes.FORBIDDEN)
        const body = (await response.json()) as {
          error: { code: string; details: { name: string; limit: unknown; current: unknown } }
        }
        expect(body.error.code).toBe('limit_exceeded')
        expect(body.error.details).toMatchObject({ name, limit })
        expect(body.error.details.current).toBeDefined()
        await projects.set('default', { [name]: null })
      }
      expect(await rows(c.db, sql`SELECT 1 FROM edge_addresses`)).toEqual([])
      expect((await projects.get('default')).usage.sandboxes).toBe(0)
    } finally {
      await c.close()
    }
  })
  test('exec and both snapshot quotas are enforced and rejected captures resume safely', async () => {
    const c = await publicContext(),
      projects = new Projects(c.boxes.projects as ProjectAdmission, c.boxes)
    try {
      const box = (await (await c.request('/sandboxes', 'POST', { image: 'alpine:3.22' })).json()) as {
        id: string
      }
      await projects.set('default', { maxExecTimeout: 1 })
      const exec = await c.request(`/sandboxes/${box.id}/exec`, 'POST', { command: ['true'], timeout: 2 })
      expect(exec.status).toBe(StatusCodes.FORBIDDEN)
      expect(((await exec.json()) as { error: { details: unknown } }).error.details).toEqual({
        name: 'maxExecTimeout',
        limit: 1,
        current: 0,
      })
      for (const name of ['maxSnapshots', 'maxSnapshotBytes'] as const) {
        await projects.set('default', { [name]: 0 })
        const response = await c.request(`/sandboxes/${box.id}/snapshots`, 'POST')
        expect(response.status).toBe(StatusCodes.FORBIDDEN)
        expect(((await response.json()) as { error: { details: { name: string } } }).error.details.name).toBe(
          name,
        )
        expect((await c.boxes.get(box.id, 'default')).state).toBe('running')
        expect((await projectUsage(c.db, 'default')).snapshots).toBe(0)
        await projects.set('default', { [name]: null })
      }
      const first = await c.request(`/sandboxes/${box.id}/snapshots`, 'POST')
      expect(first.status).toBe(StatusCodes.CREATED)
      await projects.set('default', { maxSnapshots: 1 })
      const refused = await c.request(`/sandboxes/${box.id}/snapshots`, 'POST')
      expect(refused.status).toBe(StatusCodes.FORBIDDEN)
      expect((await projectUsage(c.db, 'default')).snapshots).toBe(1)
      await c.request(`/sandboxes/${box.id}`, 'DELETE')
    } finally {
      await c.close()
    }
  })
  test('concurrent creates reserve the last unit once, stop releases it and wake checks it again', async () => {
    const c = await publicContext(),
      projects = new Projects(c.boxes.projects as ProjectAdmission, c.boxes)
    try {
      await projects.set('default', { maxUnits: 1, maxRunning: 1 })
      const attempts = await Promise.all(
        Array.from({ length: 8 }, () => c.request('/sandboxes', 'POST', { image: 'alpine:3.22' })),
      )
      expect(attempts.filter((r) => r.status === StatusCodes.CREATED)).toHaveLength(1)
      expect(attempts.filter((r) => r.status === StatusCodes.FORBIDDEN)).toHaveLength(7)
      const first = (await (attempts.find((r) => r.status === StatusCodes.CREATED) as Response).json()) as {
        id: string
      }
      expect((await projectUsage(c.db, 'default')).units).toBe(1)
      await c.request(`/sandboxes/${first.id}/stop`, 'POST')
      expect((await projectUsage(c.db, 'default')).units).toBe(0)
      const second = await c.request('/sandboxes', 'POST', { image: 'alpine:3.22' })
      expect(second.status).toBe(StatusCodes.CREATED)
      expect((await c.request(`/sandboxes/${first.id}/wake`, 'POST')).status).toBe(StatusCodes.FORBIDDEN)
      await projects.remove('default')
      expect((await c.request('/sandboxes')).status).toBe(StatusCodes.UNAUTHORIZED)
    } finally {
      await c.close()
    }
  })
  test('freeze waits for an admitted create, then refuses writes and permits reads until unfreeze', async () => {
    const c = await publicContext(),
      projects = new Projects(c.boxes.projects as ProjectAdmission, c.boxes)
    try {
      let release!: () => void, entered!: () => void
      const gate = new Promise<void>((resolve) => {
          release = resolve
        }),
        began = new Promise<void>((resolve) => {
          entered = resolve
        })
      c.nodes.once(/^PUT /, async (answer) => {
        entered()
        await gate
        return answer()
      })
      const making = c.request('/sandboxes', 'POST', { image: 'alpine:3.22' })
      await began
      const freezing = projects.freeze('default')
      release()
      const box = (await (await making).json()) as { id: string }
      await freezing
      expect((await c.boxes.get(box.id, 'default')).state).toBe('stopped')
      expect((await c.request(`/sandboxes/${box.id}`)).status).toBe(StatusCodes.OK)
      expect((await c.request(`/sandboxes/${box.id}/wake`, 'POST')).status).toBe(StatusCodes.FORBIDDEN)
      expect((await c.request(`/sandboxes/${box.id}/files/x`, 'PUT', { dataBase64: 'eA==' })).status).toBe(
        StatusCodes.FORBIDDEN,
      )
      await projects.unfreeze('default')
      expect((await c.request(`/sandboxes/${box.id}/wake`, 'POST')).status).toBe(StatusCodes.OK)
      await projects.set('default', { expiresAt: '2000-01-01T00:00:00.000Z' })
      await projects.upkeep()
      expect((await c.request('/sandboxes')).status).toBe(StatusCodes.UNAUTHORIZED)
      expect(await rows(c.db, sql`SELECT 1 FROM sandboxes WHERE deleted_at IS NULL`)).toEqual([])
    } finally {
      await c.close()
    }
  })
  test('operator routes require authentication and create a scoped key once', async () => {
    const c = await publicContext(),
      projects = new Projects(c.boxes.projects as ProjectAdmission, c.boxes),
      token = 'operator-token-which-is-long-enough'
    try {
      const app = createOperatorApi({
        db: c.db,
        runtime: c.boxes.engine,
        thresholds: THRESHOLDS,
        token,
        relocate: async () => {},
      })
      withProjects(app, projects)
      expect((await app.request('/operator/v1/projects')).status).toBe(StatusCodes.UNAUTHORIZED)
      const request = (path: string, method = 'GET', body?: unknown) =>
        app.request(`/operator/v1/projects${path}`, {
          method,
          headers: { authorization: `Bearer ${token}`, 'content-type': 'application/json' },
          ...(body ? { body: JSON.stringify(body) } : {}),
        })
      const created = await request('', 'POST', { key: true, maxUnits: 5, maxRunning: 3, maxTtl: 7200 })
      expect(created.status).toBe(StatusCodes.CREATED)
      const body = (await created.json()) as { id: string; key: { key: string } }
      expect(body.key.key.startsWith('pryx_')).toBe(true)
      expect(JSON.stringify(await (await request(`/${body.id}`)).json())).not.toContain(body.key.key)
      expect((await request(`/${body.id}/usage`)).status).toBe(StatusCodes.OK)
      expect((await request(`/${body.id}`, 'PATCH', { maxUnits: 2 })).status).toBe(StatusCodes.OK)
      expect((await request(`/${body.id}/freeze`, 'POST', {})).status).toBe(StatusCodes.OK)
      expect((await request(`/${body.id}/unfreeze`, 'POST', {})).status).toBe(StatusCodes.OK)
      expect((await request(`/${body.id}`, 'DELETE')).status).toBe(StatusCodes.OK)
    } finally {
      await c.close()
    }
  })
})
test('unit capacity uses both resources after reservations; every limit has an environment default', () => {
  expect(
    freeUnits({
      capacity: { allocatableMemoryMb: 16 * 1024 - 512, cpus: 8, reservedCpuMillis: 250 },
      running: { memoryMb: 0, cpuMillis: 0, workloads: 0 },
    }),
  ).toBe(31)
  expect(
    freeUnits({
      capacity: { allocatableMemoryMb: 16 * 1024 - 512, cpus: 2, reservedCpuMillis: 250 },
      running: { memoryMb: 0, cpuMillis: 0, workloads: 0 },
    }),
  ).toBe(7)
  for (const [name, variable] of Object.entries(LIMIT_ENV)) {
    const value =
      name === 'allowedImages' ? 'alpine:,node:' : name === 'expiresAt' ? '2030-01-01T00:00:00Z' : '7'
    const parsed = projectDefaults({ [variable]: value })
    expect(parsed[name as keyof ProjectLimits]).toEqual(
      name === 'allowedImages' ? ['alpine:', 'node:'] : name === 'expiresAt' ? value : 7,
    )
  }
})
