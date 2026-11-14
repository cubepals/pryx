// SPDX-License-Identifier: FSL-1.1-ALv2
/** Retain deployment requirements and default tier policy across shell, user-manager and upgrade
 * boundaries. Backend construction and workload validation belong to the daemon. */

import { GUARANTEES, type Guarantee } from '../../control/src/guarantees.ts'
import { type IsolationDefault, isIsolationDefault } from '../../control/src/isolation.ts'

export function localRequirements(previous: readonly Guarantee[] = [], value = process.env.PRYX_REQUIRE) {
  const names = [
    ...new Set([
      ...previous,
      ...(value
        ?.split(',')
        .map((name) => name.trim())
        .filter(Boolean) ?? []),
    ]),
  ]
  if (!names.every((name) => GUARANTEES.includes(name as Guarantee)))
    throw new Error(`Unknown deployment requirement; supported names: ${GUARANTEES.join(', ')}`)
  return names as Guarantee[]
}

export function localIsolation(
  previous?: IsolationDefault,
  value = process.env.PRYX_DEFAULT_ISOLATION,
): IsolationDefault | undefined {
  if (value === undefined) return previous
  const preference = value.includes(',') ? value.split(',').map((tier) => tier.trim()) : value
  if (!isIsolationDefault(preference))
    throw new Error('Unknown default isolation tier or empty isolation order')
  return preference
}
