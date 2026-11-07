// SPDX-License-Identifier: FSL-1.1-ALv2
/** SDK durations use generated units owned by the control contract. */
import { SDK_DEFAULTS } from './generated/defaults.ts'
export type Duration = number | `${number}${'s' | 'm' | 'h' | 'd'}`
export function seconds(value: Duration): number {
  const match = typeof value === 'string' ? /^(\d+(?:\.\d+)?)([smhd])$/.exec(value) : null
  const result =
    typeof value === 'number'
      ? value
      : match
        ? Number(match[1]) * SDK_DEFAULTS.units[match[2] as keyof typeof SDK_DEFAULTS.units]
        : NaN
  if (!Number.isFinite(result) || result <= 0)
    throw new TypeError('Duration is positive seconds or a number followed by s, m, h or d')
  return result
}
