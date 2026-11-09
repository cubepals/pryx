// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Public v1 schemas own validation and the generated API contract; node wire types stay independent. */
import { z } from 'zod'
import node from '../../openapi/node-reads.openapi.json'
import writes from '../../openapi/node-writes.openapi.json'
import { ISOLATIONS } from './isolation.ts'
import { agentSchema } from './node-contract.ts'
import type { WorkloadSpec } from './wire.ts'

const map = z.record(z.string(), z.string())
const nodeSchemas = node.components.schemas
export const networkSchema = z.fromJSONSchema(
  nodeSchemas.NetworkMode as Parameters<typeof z.fromJSONSchema>[0],
) as z.ZodType<NonNullable<WorkloadSpec['network']>>
export const resources = z.fromJSONSchema(
  nodeSchemas.Resources as Parameters<typeof z.fromJSONSchema>[0],
) as z.ZodType<WorkloadSpec['resources']>
export const storage = z.fromJSONSchema(
  nodeSchemas.Storage as Parameters<typeof z.fromJSONSchema>[0],
) as z.ZodType<WorkloadSpec['storage']>
export const port = z
  .object({
    name: z.string().regex(/^[a-z][a-z0-9-]*$/),
    port: z.number().int().min(1).max(65535),
    protocol: z.enum(['tcp', 'udp']).default('tcp'),
    audience: z.enum(['public', 'private']).default('private'),
    front: z.enum(['http', 'tls', 'port']).optional(),
    alias: z
      .string()
      .regex(/^[a-z0-9][a-z0-9-]{0,62}$/)
      .optional(),
    tls: z.boolean().optional(),
  })
  .strict()
export const makeSchema = z
  .object({
    mounts: z.array(agentSchema('MountSpec')).min(1).max(16).optional(),
    image: z.string().min(1).optional(),
    packages: z
      .array(z.string().regex(new RegExp(writes['x-pryx-wasm'].packageNamePattern)))
      .max(writes['x-pryx-wasm'].maxPackages)
      .optional(),
    snapshot: z.string().optional(),
    template: z.string().optional(),
    entrypoint: z.array(z.string()).optional(),
    resources: resources.optional(),
    units: z.number().int().positive().safe().optional(),
    storage: storage.optional(),
    ports: z.array(port).default([]),
    sparePorts: z
      .number()
      .int()
      .nonnegative()
      .max(nodeSchemas.WorkloadSpec.properties.sparePorts.maximum)
      .optional(),
    network: networkSchema.default(
      nodeSchemas.WorkloadSpec.properties.network.default as NonNullable<WorkloadSpec['network']>,
    ),
    env: map.default({}),
    secrets: map.default({}),
    ttl: z.number().positive().optional(),
    idle: z.number().positive().optional(),
    isolation: z.enum(ISOLATIONS).optional(),
    restart: z
      .object({
        policy: z.enum(nodeSchemas.RestartPolicy.oneOf.map((p) => p.const) as ['no', 'on-failure']),
        maxRetries: z.number().int().nonnegative(),
      })
      .optional(),
    labels: map.default({}),
    name: z
      .string()
      .regex(/^[a-z0-9][a-z0-9-]{0,62}$/)
      .optional(),
    metadata: z.record(z.string(), z.unknown()).optional(),
    workdir: z
      .string()
      .min(1)
      .max(4096)
      .refine((v) => v.startsWith('/') && !v.includes('\0'))
      .optional(),
    user: z
      .string()
      .regex(/^[0-9]+(?::[0-9]+)?$/)
      .optional(),
    autoDelete: z.boolean().optional(),
    egress: z
      .object({
        allow: z.array(z.string().regex(/^(?:\*\.)?[a-z0-9](?:[a-z0-9.-]*[a-z0-9])?$/)).max(128),
        deny: z
          .array(z.string().regex(/^(?:\*\.)?[a-z0-9](?:[a-z0-9.-]*[a-z0-9])?$/))
          .max(128)
          .default([]),
      })
      .strict()
      .optional(),
    start: z.boolean().default(true),
    region: z.string().default('default'),
  })
  .strict()
export const execSchema = z
  .object({
    command: z.array(z.string()).min(1),
    timeout: z.number().int().positive().default(nodeSchemas.ExecRequest.properties.timeoutSeconds.default),
    env: z
      .record(
        z.string().regex(/^[A-Za-z_][A-Za-z0-9_]*$/),
        z.string().refine((v) => !v.includes('\0')),
      )
      .default({}),
  })
  .strict()
export const snapshotSchema = z.object({
  id: z.string(),
  sizeBytes: z.number().optional(),
  at: z.string().optional(),
})
export const sandboxSchema = z.object({
  name: z.string().optional(),
  labels: map.optional(),
  metadata: z.record(z.string(), z.unknown()).optional(),
  workdir: z.string().optional(),
  user: z.string().optional(),
  idle: z.number().optional(),
  autoDelete: z.boolean().optional(),
  exit: z
    .object({ code: z.number().int(), oomKilled: z.boolean(), at: z.string().nullable().optional() })
    .optional(),
  isolation: z.enum(ISOLATIONS).optional(),
  files: z.boolean().optional(),
  agent: z.object({ url: z.string() }).optional(),
  id: z.string(),
  project: z.string(),
  units: z.number().int().positive().optional(),
  state: z.enum(['creating', 'stopped', 'starting', 'running', 'sleeping', 'resting', 'restoring', 'failed']),
  node: z.string().nullable(),
  epoch: z.number().nullable(),
  endpoints: z.array(
    z.object({
      name: z.string(),
      host: z.string(),
      port: z.number(),
      hostname: z.string().optional(),
      publicPort: z.number().optional(),
      protocol: z.string().optional(),
    }),
  ),
  usage: z.unknown(),
  issues: z.array(z.unknown()),
  createdAt: z.string(),
  updatedAt: z.string(),
  expiresAt: z.string().nullable(),
})
export const errorSchema = z.object({
  error: z.object({ code: z.string(), message: z.string(), details: z.unknown().optional() }),
})
export const execEvent = z.union([
  z.object({ type: z.literal('stdout'), data: z.string(), truncated: z.boolean() }),
  z.object({ type: z.literal('stderr'), data: z.string(), truncated: z.boolean() }),
  z.object({
    type: z.literal('exit'),
    code: z.number().nullable(),
    timedOut: z.boolean(),
    killed: z.boolean(),
    durationMs: z.number(),
  }),
  z.object({ type: z.literal('error'), error: errorSchema.shape.error }),
])
/** A template is a prepared image: mounts carry runtime credentials, so a caller passes them
 * when creating a sandbox and a template's stored spec never holds them. */
const templateMounts = 'Mounts are passed when a sandbox is created, never kept in a template'
export const templateSchema = z
  .object({
    name: z.string().regex(/^[a-z0-9][a-z0-9-]{0,62}$/),
    spec: makeSchema,
  })
  .superRefine((t, ctx) => {
    if (t.spec.mounts !== undefined)
      ctx.addIssue({ code: 'custom', path: ['spec', 'mounts'], message: templateMounts })
  })
export type Make = z.infer<typeof makeSchema>
export type Exec = z.infer<typeof execSchema>
export type PublicPort = z.infer<typeof port>
