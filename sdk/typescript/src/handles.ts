// SPDX-License-Identifier: FSL-1.1-ALv2
/** Sandbox and snapshot handles compose the stable raw API. State changes follow server responses and refresh is explicit. */
import type { MakeRequest, Pryx, Sandbox as SandboxView, Snapshot as SnapshotView } from './api.ts'
import { type Duration, seconds } from './duration.ts'
import { Files } from './files.ts'
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
  readonly files: Files
  constructor(
    private api: Pryx,
    private view: SandboxView,
  ) {
    this.files = new Files(api, view.id)
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
    this.view = await this.api.get(this.id)
    return this
  }
  run(command: string | string[], options?: RunOptions) {
    return new Run(this.api, this.id, command, options)
  }
  write(path: string, data: string | Uint8Array) {
    return this.files.write(path, data)
  }
  read(path: string) {
    return this.files.read(path)
  }
  readBytes(path: string) {
    return this.files.readBytes(path)
  }
  async snapshot() {
    return new Snapshot(this.api, await this.api.snapshot(this.id), this.id)
  }
  async snapshots() {
    return (await this.api.snapshots(this.id)).snapshots.map((view) => new Snapshot(this.api, view, this.id))
  }
  private async action(verb: Parameters<Pryx['action']>[1]) {
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
  start() {
    return this.action('start')
  }
  rest() {
    return this.action('rest')
  }
  delete() {
    return this.api.delete(this.id)
  }
  async endpoints() {
    return (await this.api.endpoints(this.id)).endpoints
  }
  logs(options?: Parameters<Pryx['logs']>[1], signal?: AbortSignal) {
    return this.api.logs(this.id, options, signal)
  }
}
