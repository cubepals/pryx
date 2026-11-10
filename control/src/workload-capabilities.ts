// SPDX-License-Identifier: FSL-1.1-ALv2
/** Required node features are checked before both spec writes and unchanged-spec wakes. */
import { StatusCodes } from 'http-status-codes'
import { NodeRefused } from './node-client.ts'
import { RuntimeFull, type RuntimeSpec } from './runtime-model.ts'

export function workloadFeatures(spec: RuntimeSpec): string[] {
  return [
    ...(spec.workdir !== undefined || spec.user !== undefined ? ['workload-options'] : []),
    ...(spec.egress !== undefined ? ['egress-hostnames'] : []),
    ...(spec.packages?.length ? ['wasm-packages'] : []),
    ...(spec.network === 'none' ? ['network-policy'] : []),
  ]
}

export function requireWorkloadFeatures(spec: RuntimeSpec, features: readonly string[]): void {
  const guarantees = spec.require?.filter((name) => !features.includes(name)) ?? []
  if (guarantees.length)
    throw new RuntimeFull('fleet', `Node lacks required guarantees: ${guarantees.join(', ')}`, guarantees)
  const missing = workloadFeatures(spec).filter((feature) => !features.includes(feature))
  if (missing.length)
    throw new NodeRefused(
      StatusCodes.CONFLICT,
      'unsupported',
      `Node lacks required features: ${missing.join(', ')}`,
      { missing },
    )
}
