// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Edge v1 schemas: routing, wake and activity only; no application protocol is represented. */
import { z } from 'zod'
import { agentViewSchema } from './node-contract.ts'
export const EdgeRoute = z.object({
  agent: z.object({ manifest: agentViewSchema, routeToken: z.string().optional() }).optional(),
  id: z.string(),
  sandbox: z.string(),
  hostname: z.string(),
  port: z.string(),
  front: z.enum(['http', 'tls', 'port']),
  publicPort: z.number().int(),
  destination: z.string(),
  epoch: z.number().int().nonnegative(),
  state: z.enum(['asleep', 'restarting', 'waking']).optional(),
})
export const EdgeRoutes = z.object({ routes: z.array(EdgeRoute) })
export type EdgeRoute = z.infer<typeof EdgeRoute>
export type EdgeRoutes = z.infer<typeof EdgeRoutes>
export const WakeRequest = z.object({ route: z.string() })
export const WakeResult = z.union([
  z.object({ outcome: z.literal('ready'), destination: z.string(), epoch: z.number().int() }),
  z.object({ outcome: z.literal('starting') }),
  z.object({ outcome: z.literal('denied'), reason: z.string() }),
])
export type WakeResult = z.infer<typeof WakeResult>
export const SessionEvent = z.object({
  id: z.string().max(128),
  sandbox: z.string(),
  epoch: z.number().int(),
  event: z.enum(['connect', 'disconnect']),
  at: z.string().optional(),
})
export const VisitReport = z.object({
  sandbox: z.string(),
  event: z.enum(['connect', 'disconnect', 'refused']),
  bytesUp: z.number().nonnegative().optional(),
  bytesDown: z.number().nonnegative().optional(),
})
export const VisitReports = z.object({ reports: z.array(VisitReport).max(200) })

export const IdleHint = z.object({ sandbox: z.string() })
export const PingWakeResult = z.object({ waking: z.boolean() })
