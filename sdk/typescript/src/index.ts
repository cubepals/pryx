// SPDX-License-Identifier: FSL-1.1-ALv2
/** Public lifecycle facade with one handle surface.
 * Parts (`./`): `transport.ts` owns private HTTP; `agent.ts` owns process/terminal capabilities;
 * `handles.ts` owns sandbox/snapshot identities; `run.ts` owns eager observations; `files.ts`
 * owns binary files; `git.ts`, `shell.ts`, `template.ts`, `webhook.ts` own convenience handles;
 * `git-proxy.ts` owns the transient wasm bridge; `local-files.ts` owns caller disk streaming; `logs.ts` owns agent log decoding; `duration.ts` parses durations.
 * `kernels.ts` owns persistent language contexts and rich result collection.
 * `provider.ts` is the separate Rivet sandbox adapter entry point.
 */

import { StatusCodes } from 'http-status-codes'
import type { components } from './generated/api.ts'
import { makeOptions, Sandbox, type SandboxOptions } from './handles.ts'
import type { RunOptions } from './run.ts'
import { buildTemplate, type TemplateOptions } from './template.ts'
import { PryxError, type Sandbox as SandboxView, Transport } from './transport.ts'
import { Webhook, type WebhookOptions, type WebhookView } from './webhook.ts'

export type { StartOptions } from './agent.ts'
export { Process, Terminal } from './agent.ts'
export type { Duration } from './duration.ts'
export type { FileData } from './files.ts'
export { Files } from './files.ts'
export type { GitResult } from './git.ts'
export type { SandboxOptions } from './handles.ts'
export { Sandbox, Snapshot } from './handles.ts'
export type {
  Kernel,
  KernelError,
  KernelEvent,
  KernelOptions,
  KernelResult,
  KernelRunOptions,
  KernelView,
} from './kernels.ts'
export type { LogRecord, LogsOptions } from './logs.ts'
export type { Chunk, RunOptions, RunResult } from './run.ts'
export { CommandStream } from './run.ts'
export type { TemplateOptions, TemplateView } from './template.ts'
export type {
  ExecEvent,
  ExecRequest,
  MakeRequest,
  Sandbox as SandboxView,
  Snapshot as SnapshotView,
} from './transport.ts'
export { PryxError } from './transport.ts'
export type { WebhookOptions, WebhookView } from './webhook.ts'
export type PryxOptions = { url: string; key: string; fetch?: typeof fetch }

export class Pryx {
  #http: Transport
  constructor(options: PryxOptions) {
    this.#http = new Transport(options)
  }
  async sandbox(options: string | SandboxOptions = {}, lookup: { wake?: boolean } = {}) {
    if (
      typeof options === 'object' &&
      options.repo &&
      !options.image &&
      !options.template &&
      !options.snapshot &&
      (!options.isolation || options.isolation === 'container')
    ) {
      const defaults = (await this.project()).defaults
      if ((options.isolation ?? defaults?.isolation) === 'container')
        options = { ...options, image: defaults?.repoImage }
      if ((options.isolation ?? defaults?.isolation) === 'wasm')
        options = { ...options, packages: [...new Set([...(options.packages ?? []), 'git'])] }
    }
    if (typeof options === 'object' && options.repo && options.isolation === 'wasm')
      options = { ...options, packages: [...new Set([...(options.packages ?? []), 'git'])] }
    let view: SandboxView
    let existing = typeof options === 'string'
    if (
      typeof options === 'object' &&
      options.name &&
      Object.keys(options).every((key) => ['name', 'wake'].includes(key))
    ) {
      const matches = await this.sandboxes({ name: options.name })
      const match = matches[0]
      if (!match) throw new PryxError(StatusCodes.NOT_FOUND, 'not_found', 'No sandbox with that name')
      view = match.toJSON()
      existing = true
    } else
      view =
        typeof options === 'string'
          ? await this.#http.request<SandboxView>(`/v1/sandboxes/${encodeURIComponent(options)}`)
          : await this.#http.request<SandboxView>(
              '/v1/sandboxes',
              'POST',
              makeOptions(options),
              options.idempotencyKey ?? crypto.randomUUID(),
            )
    const box = new Sandbox(this.#http, view)
    if (
      existing &&
      (lookup.wake ?? (typeof options === 'object' ? options.wake : undefined) ?? true) &&
      ['sleeping', 'resting'].includes(box.state)
    )
      await box.wake()
    if (typeof options === 'object' && options.repo) {
      try {
        await box.git.clone(options.repo, { branch: options.branch, token: options.token })
      } catch (error) {
        if (error instanceof PryxError)
          throw new PryxError(error.status, error.code, error.message, {
            sandboxId: box.id,
            cause: error.details,
          })
        throw new PryxError(
          0,
          'repo_failed',
          error instanceof Error ? error.message : 'Repository preparation failed',
          { sandboxId: box.id },
        )
      }
    }
    return box
  }
  async sandboxes(
    filters: { labels?: Record<string, string>; state?: SandboxView['state']; name?: string } = {},
  ) {
    const query = new URLSearchParams(
      Object.fromEntries(
        Object.entries({
          ...filters,
          labels: filters.labels ? JSON.stringify(filters.labels) : undefined,
        }).filter((entry): entry is [string, string] => entry[1] !== undefined),
      ),
    )
    return (
      await this.#http.request<{ sandboxes: SandboxView[] }>(`/v1/sandboxes${query.size ? `?${query}` : ''}`)
    ).sandboxes.map((view) => new Sandbox(this.#http, view))
  }
  template(options: TemplateOptions) {
    return buildTemplate(this.#http, (options) => this.sandbox(options), options)
  }
  async templates() {
    return (await this.#http.request<{ templates: unknown[] }>('/v1/templates')).templates
  }
  async webhook(options: WebhookOptions) {
    const { secret, ...view } = await this.#http.request<WebhookView & { secret: string }>(
      '/v1/webhooks',
      'POST',
      options,
    )
    return new Webhook(this.#http, view, secret)
  }
  async webhooks() {
    return (await this.#http.request<{ webhooks: WebhookView[] }>('/v1/webhooks')).webhooks
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

export type { Mount, SshAccess, SshAddress, ToolAccess } from './tools.ts'
