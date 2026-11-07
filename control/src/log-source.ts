// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

import type { RuntimeHandle } from './runtime-model.ts'

/** A raw line of workload output. What it means is application knowledge, decided elsewhere. */
export interface LogLine {
  at: Date
  text: string
}

/**
 * A workload's output, found by the handle its runtime issued: every runtime brings its own, and
 * main.node.ts pairs them. A tail follows one workload through its restarts (§15.5).
 */
export interface LogSource {
  recent(handle: RuntimeHandle, limit: number): Promise<LogLine[]>
  tail(handle: RuntimeHandle, signal: AbortSignal): AsyncIterable<LogLine>
}
