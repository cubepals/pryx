// SPDX-License-Identifier: FSL-1.1-ALv2
/** Retain deployment requirements across shell, user-manager and upgrade boundaries. */
import { GUARANTEES, type Guarantee } from '../../control/src/guarantees.ts'

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
