// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Resolve committed Rust schema references for runtime validation without copying their shapes. */
import { z } from 'zod'
import agent from '../../agent/openapi.json'
import writes from '../../openapi/node-writes.openapi.json'
import type { AgentView, FileInfo } from './wire.ts'

function resolve(value: unknown, definitions: Record<string, unknown> = writes.components.schemas): unknown {
  if (Array.isArray(value)) return value.map((item) => resolve(item, definitions))
  if (value !== null && typeof value === 'object') {
    const fields = value as Record<string, unknown>
    if (typeof fields.$ref === 'string') {
      const name = fields.$ref.replace('#/components/schemas/', '')
      const schema = definitions[name]
      if (!schema) throw new Error('Unknown node schema reference')
      return resolve(schema, definitions)
    }
    return Object.fromEntries(Object.entries(fields).map(([key, item]) => [key, resolve(item, definitions)]))
  }
  return value
}
export const agentViewSchema = z.fromJSONSchema(
  resolve(writes.components.schemas.AgentView) as Parameters<typeof z.fromJSONSchema>[0],
) as z.ZodType<AgentView>
export const fileInfoSchema = z.fromJSONSchema(
  resolve(writes.components.schemas.FileInfo) as Parameters<typeof z.fromJSONSchema>[0],
) as z.ZodType<FileInfo>
export const agentClaimsSchema = z.fromJSONSchema(
  resolve(writes.components.schemas.Claims) as Parameters<typeof z.fromJSONSchema>[0],
)
export function agentSchema(name: keyof typeof agent.components.schemas) {
  return z.fromJSONSchema(
    resolve(agent.components.schemas[name], agent.components.schemas) as Parameters<
      typeof z.fromJSONSchema
    >[0],
  )
}
