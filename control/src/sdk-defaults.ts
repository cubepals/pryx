// SPDX-License-Identifier: FSL-1.1-ALv2
/** Public SDK convenience defaults. Raw make requests retain image entrypoints. */
import { DAY_S, HOUR_S, MINUTE_S } from './durations.ts'
export const SDK_DEFAULTS = {
  units: { s: 1, m: MINUTE_S, h: HOUR_S, d: DAY_S },
  fileChunkBytes: 32 * 1024,
  entrypoint: [
    'sh',
    '-c',
    'trap \'kill "$child"; exit 0\' TERM INT; tail -f /dev/null & child=$!; wait "$child"',
  ],
}
