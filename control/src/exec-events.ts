// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Encode the persisted bounded exec answer using the existing public event contract. */
import type { z } from 'zod'
import type { execEvent } from './api-schemas.ts'
import type { ExecResponse } from './wire.ts'

export async function* responseEvents(result: ExecResponse): AsyncIterable<z.infer<typeof execEvent>> {
  for (const type of ['stdout', 'stderr'] as const)
    if (result[type]) yield { type, data: result[type], truncated: result[`${type}Truncated`] }
  yield {
    type: 'exit',
    code: result.exitCode,
    timedOut: result.timedOut,
    killed: result.killed,
    durationMs: result.durationMs,
  }
}
