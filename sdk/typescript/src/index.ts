// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Public SDK facade composes lifecycle and direct sandbox operations.
 * Parts (`./`):
 * - `api.ts`: authenticated public transport.
 * - `agent.ts`: direct processes, terminals and tokens.
 * - `handles.ts`: sandbox and snapshot identities.
 * - `run.ts`: command values and output.
 * - `files.ts`: persistent file transports.
 * - `duration.ts`: duration parsing.
 */
import { Agent } from './agent.ts'
import { Pryx as Api } from './api.ts'
import { makeOptions, Sandbox, type SandboxOptions } from './handles.ts'
import type { RunOptions } from './run.ts'

export type { StartOptions } from './agent.ts'
export { Agent, Process, Terminal } from './agent.ts'
export type {
  ExecEvent,
  ExecRequest,
  MakeRequest,
  Sandbox as SandboxView,
  Snapshot as SnapshotView,
} from './api.ts'
export { ndjson, PryxError } from './api.ts'
export type { Duration } from './duration.ts'
export { seconds } from './duration.ts'
export type { SandboxOptions } from './handles.ts'
export { Sandbox, Snapshot } from './handles.ts'
export type { Chunk, RunOptions, RunResult } from './run.ts'
export class Pryx extends Api {
  agent(id: string) {
    return new Agent(this.api, id)
  }
  get api(): Api {
    return this
  }
  constructor(
    options: { url: string; key: string; fetch?: typeof fetch } | ConstructorParameters<typeof Api>[0],
  ) {
    super('url' in options ? { baseUrl: options.url, apiKey: options.key, fetch: options.fetch } : options)
  }
  async sandbox(options: string | SandboxOptions = {}) {
    return new Sandbox(
      this.api,
      typeof options === 'string'
        ? await this.api.get(options)
        : await this.api.make(makeOptions(options), options.idempotencyKey ?? crypto.randomUUID()),
    )
  }
  async sandboxes() {
    return (await this.api.list()).sandboxes.map((view) => new Sandbox(this.api, view))
  }
  async run(image: string, command: string | string[], options?: RunOptions) {
    const box = await this.sandbox({ image })
    try {
      return await box.run(command, options)
    } finally {
      await box.delete()
    }
  }
}
