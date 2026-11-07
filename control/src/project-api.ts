// SPDX-License-Identifier: FSL-1.1-ALv2
/** Project routes on the authenticated operator listener; no account or billing policy lives here. */
import type { Hono } from 'hono'
import { StatusCodes } from 'http-status-codes'
import { z } from 'zod'
import { limitsSchema } from './project-model.ts'
import type { Projects } from './projects.ts'

export const PROJECT_BASE = '/operator/v1/projects'
const name = z.string().trim().min(1).max(128)
export function withProjects(app: Hono, projects: Projects) {
  app.post(PROJECT_BASE, async (c) => {
    const {
      name: label = 'Project',
      key = false,
      ...limits
    } = limitsSchema.extend({ name: name.optional(), key: z.boolean().optional() }).parse(await c.req.json())
    return c.json(await projects.create(label, limits, key), StatusCodes.CREATED)
  })
  app.get(PROJECT_BASE, async (c) => c.json({ projects: await projects.list() }))
  app.get(`${PROJECT_BASE}/:id`, async (c) => c.json(await projects.get(c.req.param('id'))))
  app.get(`${PROJECT_BASE}/:id/usage`, async (c) => {
    const p = await projects.get(c.req.param('id'))
    return c.json({ limits: p.limits, usage: p.usage })
  })
  app.patch(`${PROJECT_BASE}/:id`, async (c) => {
    const { name: label, ...limits } = limitsSchema
      .extend({ name: name.optional() })
      .parse(await c.req.json())
    return c.json(await projects.set(c.req.param('id'), limits, label))
  })
  app.post(`${PROJECT_BASE}/:id/freeze`, async (c) => c.json(await projects.freeze(c.req.param('id'))))
  app.post(`${PROJECT_BASE}/:id/unfreeze`, async (c) => c.json(await projects.unfreeze(c.req.param('id'))))
  app.delete(`${PROJECT_BASE}/:id`, async (c) => c.json(await projects.remove(c.req.param('id'))))
}
