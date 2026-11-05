// SPDX-License-Identifier: FSL-1.1-ALv2
/** Sandbox and snapshot handles compose the stable raw API. State changes follow server responses and refresh is explicit. */

import { Agent, output, type Process, type StartOptions } from './agent.ts'
import type { MakeRequest, Pryx, Sandbox as SandboxView, Snapshot as SnapshotView } from './api.ts'
import { type Duration, seconds } from './duration.ts'
import { type FileData, Files } from './files.ts'
import { SDK_DEFAULTS } from './generated/defaults.ts'
import { Run, type RunOptions } from './run.ts'
export type SandboxOptions = Omit<MakeRequest, 'idle' | 'ttl'> & {
  idle?: Duration
  ttl?: Duration
  idempotencyKey?: string
}
export function makeOptions(options: SandboxOptions): MakeRequest {
  const { idle, ttl, idempotencyKey: _, ...body } = options
  return {
    ...body,
    ...(idle === undefined ? {} : { idle: seconds(idle) }),
    ...(ttl === undefined ? {} : { ttl: seconds(ttl) }),
    ...(body.image && !body.entrypoint ? { entrypoint: [...SDK_DEFAULTS.entrypoint] } : {}),
  }
}
export class Snapshot {
  constructor(
    private api: Pryx,
    readonly view: SnapshotView,
    private sandbox: string,
  ) {}
  get id() {
    return this.view.id
  }
  async fork(options: Omit<SandboxOptions, 'image' | 'snapshot' | 'template'> = {}) {
    return new Sandbox(
      this.api,
      await this.api.fork(this.id, makeOptions(options), options.idempotencyKey ?? crypto.randomUUID()),
    )
  }
  delete() {
    return this.api.deleteSnapshot(this.sandbox, this.id)
  }
}
export class Sandbox {
  readonly agent: Agent
  readonly files: Files
  constructor(
    private api: Pryx,
    private view: SandboxView,
  ) {
    this.agent = new Agent(api, view.id)
    this.files = new Files(api, view.id, { agent: this.agent, view: () => this.view })
  }
  get id() {
    return this.view.id
  }
  get state() {
    return this.view.state
  }
  get units() {
    return this.view.units
  }
  async refresh() {
    this.agent.invalidate()
    this.view = await this.api.get(this.id)
    return this
  }
  run(command: string | string[], options?: RunOptions) {
    const agent = this.agent
    return new Run(
      this.api,
      this.id,
      command,
      options,
      this.view.agent
        ? async function* (request, key) {
            const process = await agent.start(command, {
              env: request.env,
              cwd: options?.cwd,
              timeout: request.timeout ?? SDK_DEFAULTS.runTimeoutSeconds,
              idempotencyKey: key,
            })
            yield* output(process.events())
          }
        : undefined,
    )
  }
  write(path: string, data: FileData) {
    return this.files.write(path, data)
  }
  read(path: string) {
    return this.files.read(path)
  }
  readBytes(path: string) {
    return this.files.readBytes(path)
  }
  readStream(path: string) {
    return this.files.readStream(path)
  }
  async snapshot() {
    return new Snapshot(this.api, await this.api.snapshot(this.id), this.id)
  }
  async snapshots() {
    return (await this.api.snapshots(this.id)).snapshots.map((view) => new Snapshot(this.api, view, this.id))
  }
  private async action(verb: Parameters<Pryx['action']>[1]) {
    this.agent.invalidate()
    this.view = (await this.api.action(this.id, verb)).sandbox
    return this
  }
  sleep() {
    return this.action('sleep')
  }
  wake() {
    return this.action('wake')
  }
  stop() {
    return this.action('stop')
  }
  start(): Promise<this>
  start(command: string | string[], options?: StartOptions): Promise<Process>
  start(command?: string | string[], options?: StartOptions) {
    return command === undefined ? this.action('start') : this.agent.start(command, options)
  }
  terminal(options?: Parameters<Agent['terminal']>[0]) {
    return this.agent.terminal(options)
  }
  async expose(port: number) {
    return (await this.agent.expose(port)).url
  }
  unexpose(port: number) {
    return this.agent.unexpose(port)
  }
  async network(mode: 'internet' | 'none') {
    this.agent.invalidate()
    this.view = await this.api.request<SandboxView>(
      `/v1/sandboxes/${encodeURIComponent(this.id)}/network`,
      'POST',
      { network: mode },
    )
    return this
  }
  rest() {
    return this.action('rest')
  }
  delete() {
    this.agent.invalidate()
    return this.api.delete(this.id)
  }
  async endpoints() {
    return (await this.api.endpoints(this.id)).endpoints
  }
  logs(options?: Parameters<Pryx['logs']>[1], signal?: AbortSignal) {
    return this.api.logs(this.id, options, signal)
  }
}
