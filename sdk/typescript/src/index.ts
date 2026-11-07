// SPDX-License-Identifier: FSL-1.1-ALv2
/** Public lifecycle facade with one handle surface.
 * Parts (`./`): `transport.ts` owns private HTTP; `agent.ts` owns process/terminal capabilities;
 * `handles.ts` owns sandbox/snapshot identities; `run.ts` owns eager observations; `files.ts`
 * owns binary files; `logs.ts` owns agent log decoding; `duration.ts` parses durations.
 */

import type { components } from './generated/api.ts'
import { makeOptions, Sandbox, type SandboxOptions } from './handles.ts'
import type { RunOptions } from './run.ts'
import { type Sandbox as SandboxView, Transport } from './transport.ts'

export type { StartOptions } from './agent.ts'
export { Process, Terminal } from './agent.ts'
export type { Duration } from './duration.ts'
export type { FileData } from './files.ts'
export { Files } from './files.ts'
export type { SandboxOptions } from './handles.ts'
export { Sandbox, Snapshot } from './handles.ts'
export type { LogRecord, LogsOptions } from './logs.ts'
export type { Chunk, CommandStream, RunOptions, RunResult } from './run.ts'
export type {
  ExecEvent,
  ExecRequest,
  MakeRequest,
  Sandbox as SandboxView,
  Snapshot as SnapshotView,
} from './transport.ts'
export { PryxError } from './transport.ts'
export type PryxOptions = { url: string; key: string; fetch?: typeof fetch }

export class Pryx {
  #http: Transport
  constructor(options: PryxOptions) {
    this.#http = new Transport(options)
  }
  async sandbox(options: string | SandboxOptions = {}) {
    const view =
      typeof options === 'string'
        ? await this.#http.request<SandboxView>(`/v1/sandboxes/${encodeURIComponent(options)}`)
        : await this.#http.request<SandboxView>(
            '/v1/sandboxes',
            'POST',
            makeOptions(options),
            options.idempotencyKey ?? crypto.randomUUID(),
          )
    return new Sandbox(this.#http, view)
  }
  async sandboxes() {
    return (await this.#http.request<{ sandboxes: SandboxView[] }>('/v1/sandboxes')).sandboxes.map(
      (view) => new Sandbox(this.#http, view),
    )
  }
  project() {
    return this.#http.request<components['schemas']['Project']>('/v1/project')
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
