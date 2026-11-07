// SPDX-License-Identifier: FSL-1.1-ALv2
/** Project configuration and unit accounting. Node wire schemas remain owned by Rust. */
import { z } from 'zod'
import { SECOND_MS } from './durations.ts'
import type { Isolation } from './isolation.ts'
import type { NodeSummary } from './registry/summaries.ts'
import type { WorkloadSpec } from './wire.ts'

export const UNIT_DEFAULTS = { memoryMb: 512, cpuMillis: 250 } as const
const count = z.number().int().nonnegative().safe().nullable().optional()
export const limitsSchema = z
  .object({
    maxUnits: count,
    maxSandboxes: count,
    maxRunning: count,
    maxUnitsPerSandbox: count,
    maxTtl: count,
    maxIdle: count,
    maxExecTimeout: count,
    maxSnapshots: count,
    maxSnapshotBytes: count,
    maxEgressPorts: count,
    allowedImages: z.array(z.string().min(1)).nullable().optional(),
    expiresAt: z.iso.datetime({ offset: true }).nullable().optional(),
  })
  .strict()
export type ProjectLimits = z.infer<typeof limitsSchema>
export type UnitSize = { memoryMb: number; cpuMillis: number }
export type ProjectSpec = WorkloadSpec & {
  image: string
  isolation?: Isolation
  units?: number
  ttl?: number
  idle?: number
}
export const LIMIT_COLUMNS = {
  maxUnits: 'max_units',
  maxSandboxes: 'max_sandboxes',
  maxRunning: 'max_running',
  maxUnitsPerSandbox: 'max_units_per_sandbox',
  maxTtl: 'max_ttl',
  maxIdle: 'max_idle',
  maxExecTimeout: 'max_exec_timeout',
  maxSnapshots: 'max_snapshots',
  maxSnapshotBytes: 'max_snapshot_bytes',
  maxEgressPorts: 'max_egress_ports',
  allowedImages: 'allowed_images',
  expiresAt: 'expires_at',
} as const
export const LIMIT_ENV = Object.fromEntries(
  Object.keys(LIMIT_COLUMNS).map((key) => [
    key,
    `PRYX_PROJECT_${key.replace(/[A-Z]/g, (c) => `_${c}`).toUpperCase()}`,
  ]),
) as Record<keyof ProjectLimits, string>
export function projectDefaults(env: Record<string, string | undefined>): ProjectLimits {
  return limitsSchema.parse(
    Object.fromEntries(
      Object.entries(LIMIT_ENV).flatMap(([key, variable]) => {
        const value = env[variable]
        if (value === undefined) return []
        return [
          [
            key,
            key === 'allowedImages'
              ? value.split(',').filter(Boolean)
              : key === 'expiresAt'
                ? value
                : Number(value),
          ],
        ]
      }),
    ),
  )
}
export function unitsFor(spec: Pick<ProjectSpec, 'resources'>, size: UnitSize = UNIT_DEFAULTS) {
  return Math.max(
    1,
    Math.ceil(spec.resources.memoryMb / size.memoryMb),
    Math.ceil((spec.resources.cpuMillis ?? 0) / size.cpuMillis),
    Math.ceil((spec.resources.cpuReservationMillis ?? 0) / size.cpuMillis),
  )
}
export function unitSpec(spec: ProjectSpec, size: UnitSize = UNIT_DEFAULTS): ProjectSpec {
  const units = spec.units ?? unitsFor(spec, size)
  return {
    ...spec,
    units,
    resources: {
      ...spec.resources,
      memoryMb: units * size.memoryMb,
      cpuReservationMillis: units * size.cpuMillis,
      ...(spec.resources.cpuMillis != null || (spec.isolation ?? 'container') === 'container'
        ? { cpuMillis: units * size.cpuMillis }
        : {}),
    },
  }
}
export function freeUnits(node: Pick<NodeSummary, 'capacity' | 'running'>, unit: UnitSize = UNIT_DEFAULTS) {
  const memory = (node.capacity.allocatableMemoryMb ?? 0) - node.running.memoryMb
  const cpu =
    (node.capacity.cpus ?? 0) * SECOND_MS - (node.capacity.reservedCpuMillis ?? 0) - node.running.cpuMillis
  return Math.max(0, Math.min(Math.floor(memory / unit.memoryMb), Math.floor(cpu / unit.cpuMillis)))
}
export class ProjectError extends Error {
  constructor(
    readonly code: string,
    message: string,
    readonly details?: object,
  ) {
    super(message)
  }
}
export function exceeded(name: keyof ProjectLimits, limit: unknown, current: unknown): never {
  throw new ProjectError('limit_exceeded', `Project limit ${name} exceeded`, { name, limit, current })
}
export function checkLimit(limits: ProjectLimits, name: keyof ProjectLimits, current: number, added = 0) {
  const limit = limits[name]
  if (typeof limit === 'number' && current + added > limit) exceeded(name, limit, current)
}
