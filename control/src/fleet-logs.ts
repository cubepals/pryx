// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

import type { LogLine, LogSource } from '../../app/ports/platform.ts'
import type { RuntimeHandle } from '../../app/ports/runtime.ts'
import type { FleetRuntime } from './fleet-runtime.ts'
import { decodeHandle } from './handle.ts'
import type { NodeClient } from './node-client.ts'
import type { LogRecord } from './wire.ts'

/**
 * A fleet server's output, from blocklyd on its node: NDJSON, one `LogRecord` per line, and a
 * follow that continues across the workload's restarts (blocklyd's docs/protocol.md, logs).
 */
export class FleetLogSource implements LogSource {
  readonly #runtime: FleetRuntime
  readonly #nodes: NodeClient

  constructor(runtime: FleetRuntime, nodes: NodeClient) {
    this.#runtime = runtime
    this.#nodes = nodes
  }

  async recent(handle: RuntimeHandle, limit: number): Promise<LogLine[]> {
    const node = this.#runtime.nodeAddress(handle)
    if (node === null) return []
    const lines: LogLine[] = []
    const path = `/v1/workloads/${decodeHandle(handle).key}/logs?tail=${Math.min(Math.max(limit, 1), 10_000)}`
    for await (const line of this.#nodes.lines(node, path, AbortSignal.timeout(30_000))) {
      const entry = parse(line)
      if (entry !== null) lines.push(entry)
    }
    return lines
  }

  async *tail(handle: RuntimeHandle, signal: AbortSignal): AsyncIterable<LogLine> {
    const node = this.#runtime.nodeAddress(handle)
    if (node === null) return
    const path = `/v1/workloads/${decodeHandle(handle).key}/logs?tail=0&follow=true`
    for await (const line of this.#nodes.lines(node, path, signal)) {
      const entry = parse(line)
      if (entry !== null) yield entry
    }
  }
}

function parse(text: string): LogLine | null {
  let entry: LogRecord
  try {
    entry = JSON.parse(text) as LogRecord
  } catch {
    return null
  }
  if (!('line' in entry)) return null
  return { at: entry.ts ? new Date(entry.ts) : new Date(), text: entry.line }
}
