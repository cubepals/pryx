// SPDX-License-Identifier: FSL-1.1-ALv2
/** Required node features are checked before both spec writes and unchanged-spec wakes. */

import { StatusCodes } from 'http-status-codes'
import contract from '../../openapi/node-writes.openapi.json'
import { guarantees as tierGuarantees } from './guarantees.ts'
import { nodeFeature, nodeIsolations } from './isolation.ts'
import { NodeRefused } from './node-client.ts'
import { RuntimeFull, type RuntimeSpec } from './runtime-model.ts'
import { TOOL_NETWORK_PROBLEM, toolFeature, toolFeatures } from './tool-capabilities.ts'

export function workloadFeatures(spec: RuntimeSpec): string[] {
  if (spec.network === 'none' && spec.requiredTools?.length)
    throw new NodeRefused(StatusCodes.UNPROCESSABLE_ENTITY, 'invalid_request', TOOL_NETWORK_PROBLEM, {
      field: 'requiredTools',
    })
  return [
    ...toolFeatures(spec.requiredTools),
    ...(spec.resources.allowUnenforcedMemory != null
      ? [contract['x-pryx-resources'].unenforcedMemoryFeature]
      : []),
    ...(spec.workdir !== undefined || spec.user !== undefined ? ['workload-options'] : []),
    ...(spec.egress !== undefined ? ['egress-hostnames'] : []),
    ...(spec.packages?.length ? ['wasm-packages'] : []),
    ...(spec.network === 'none' ? [contract['x-pryx-network'].policyFeature] : []),
  ]
}

export function requireWorkloadFeatures(spec: RuntimeSpec, features: readonly string[]): void {
  if (spec.isolation && !nodeIsolations(features).includes(spec.isolation))
    throw new RuntimeFull('fleet', `Node does not serve the ${spec.isolation} tier`, [], spec.isolation)
  const guarantees = spec.require?.filter((name) => !tierGuarantees(features, spec.isolation)[name]) ?? []
  if (guarantees.length)
    throw new RuntimeFull('fleet', `Node lacks required guarantees: ${guarantees.join(', ')}`, guarantees)
  const missingTools =
    spec.requiredTools?.filter((tool) => !nodeFeature(features, toolFeature(tool), spec.isolation)) ?? []
  if (missingTools.length)
    throw new RuntimeFull(
      'fleet',
      `Node lacks required features: ${toolFeatures(missingTools).join(', ')}`,
      [],
      spec.isolation,
      missingTools,
    )
  const missing = workloadFeatures(spec).filter((feature) => !nodeFeature(features, feature, spec.isolation))
  if (missing.length)
    throw new NodeRefused(
      StatusCodes.CONFLICT,
      'unsupported',
      `Node lacks required features: ${missing.join(', ')}`,
      { missing },
    )
}
