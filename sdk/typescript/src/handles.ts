// SPDX-License-Identifier: FSL-1.1-ALv2
/** Sandbox and snapshot handles compose the stable raw API. State changes follow server responses and refresh is explicit. */

import { Agent, type Process, type StartOptions } from './agent.ts'
import { type Duration, seconds } from './duration.ts'
import { type FileData, Files } from './files.ts'
import { SDK_DEFAULTS } from './generated/defaults.ts'
import { Git } from './git.ts'
import { CommandStream, type RunOptions } from './run.ts'
import { Shell } from './shell.ts'
import type { MakeRequest, Sandbox as SandboxView, Snapshot as SnapshotView, Transport } from './transport.ts'
import { ndjson, PryxError } from './transport.ts'
export type SandboxOptions = Omit<MakeRequest, 'idle' | 'ttl'> & {
  repo?: string
  branch?: string
  token?: string
  wake?: boolean
  idle?: Duration
  ttl?: Duration
  idempotencyKey?: string
}
export function makeOptions(options: SandboxOptions): MakeRequest {
  const {
    idle,
    ttl,
    idempotencyKey: _,
    repo: _repo,
    branch: _branch,
    token: _token,
    wake: _wake,
    ...body
  } = options
  return {
    ...body,
    ...(idle === undefined ? {} : { idle: seconds(idle) }),
    ...(ttl === undefined ? {} : { ttl: seconds(ttl) }),
    ...(body.image && !body.entrypoint ? { entrypoint: [...SDK_DEFAULTS.entrypoint] } : {}),
  }
}
export class Snapshot {
  #api: Transport
  constructor(
    api: Transport,
    readonly view: SnapshotView,
    private sandbox: string,
  ) {
    this.#api = api
  }
  get id() {
    return this.view.id
  }
  async fork(options: Omit<SandboxOptions, 'image' | 'snapshot' | 'template'> = {}) {
    return new Sandbox(
      this.#api,
      await this.#api.request<SandboxView>(
        `/v1/snapshots/${encodeURIComponent(this.id)}/sandboxes`,
        'POST',
        makeOptions(options),
        options.idempotencyKey ?? crypto.randomUUID(),
      ),
    )
  }
  delete() {
    return this.#api.request<{ existed: boolean }>(
      `/v1/sandboxes/${encodeURIComponent(this.sandbox)}/snapshots/${encodeURIComponent(this.id)}`,
      'DELETE',
    )
  }
}
export class Sandbox {
  #agent: Agent
  readonly files: Files
  readonly git: Git
  #api: Transport
  constructor(
    api: Transport,
    private view: SandboxView,
  ) {
    this.#api = api
    this.#agent = new Agent(api, view.id)
    this.git = new Git(
      (command, options) => this.run(command, options),
      (command, options) => this.spawn(command, options),
      this.isolation,
    )
    this.files = new Files(api, view.id, { agent: this.#agent, view: () => this.view })
  }
  get id() {
    return this.view.id
  }
  get state() {
    return this.view.state
  }
  get isolation() {
    return this.view.isolation
  }
  get units() {
    return this.view.units
  }
  async mcpUrl() {
    const credential = await this.#agent.token()
    const url = new URL(`${credential.url.replace(/\/$/, '')}/_pryx/mcp`)
    url.searchParams.set('token', credential.token)
    return url.toString()
  }
  async refresh() {
    this.#agent.invalidate()
    this.view = await this.#api.request<SandboxView>(this.path())
    return this
  }
  private path(suffix = '') {
    return `/v1/sandboxes/${encodeURIComponent(this.id)}${suffix}`
  }
  private command(command: string | string[], options: RunOptions = {}) {
    return this.#agent.start(command, {
      cwd: this.view.workdir,
      timeout: SDK_DEFAULTS.runTimeoutSeconds,
      ...options,
    })
  }
  run(command: string | string[], options?: RunOptions) {
    return new CommandStream(this.command(command, options), false).result()
  }
  stream(command: string | string[], options?: RunOptions) {
    return new CommandStream(this.command(command, options))
  }
  spawn(command: string | string[], options?: StartOptions): Promise<Process> {
    return this.#agent.start(command, { cwd: this.view.workdir, ...options })
  }
  processes() {
    return this.#agent.processes()
  }
  killAll() {
    return this.#agent.requestV2<{ killed: string[] }>('/_pryx/processes/kill-all', 'POST')
  }
  async shell(options: { cwd?: string; env?: Record<string, string> } = {}) {
    const view = await this.#agent.requestV2<{ id: string }>('/_pryx/shells', 'POST', {
      cwd: this.view.workdir,
      ...options,
    })
    return new Shell(this.#agent, view.id)
  }
  sync(localDir: string, remoteDir: string) {
    return this.files.sync(localDir, remoteDir)
  }
  toJSON() {
    return this.view
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
    return new Snapshot(
      this.#api,
      await this.#api.request<SnapshotView>(this.path('/snapshots'), 'POST'),
      this.id,
    )
  }
  async snapshots() {
    return (await this.#api.request<{ snapshots: SnapshotView[] }>(this.path('/snapshots'))).snapshots.map(
      (view) => new Snapshot(this.#api, view, this.id),
    )
  }
  private async action(verb: 'start' | 'stop' | 'sleep' | 'wake' | 'rest') {
    this.#agent.invalidate()
    this.view = (await this.#api.request<{ sandbox: SandboxView }>(this.path(`/${verb}`), 'POST')).sandbox
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
  terminal(options?: Parameters<Agent['terminal']>[0]) {
    return this.#agent.terminal(options)
  }
  async expose(port: number) {
    return (await this.#agent.expose(port)).url
  }
  unexpose(port: number) {
    return this.#agent.unexpose(port)
  }
  async network(mode: 'internet' | 'none') {
    this.#agent.invalidate()
    this.view = await this.#api.request<SandboxView>(
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
    this.#agent.invalidate()
    return this.#api.request<{ existed: boolean }>(this.path(), 'DELETE')
  }
  async endpoints() {
    return (await this.#api.request<{ endpoints: SandboxView['endpoints'] }>(this.path('/endpoints')))
      .endpoints
  }
  logs(options: import('./logs.ts').LogsOptions = {}, signal?: AbortSignal) {
    return this.#agent.logs(options, signal)
  }
  private async lifetime(body: { keepAlive?: number; ttl?: number; idle?: number }) {
    this.view = await this.#api.request<SandboxView>(this.path('/lifetime'), 'POST', body)
    return this
  }
  keepAlive(duration: Duration) {
    return this.lifetime({ keepAlive: seconds(duration) })
  }
  setTtl(duration: Duration) {
    return this.lifetime({ ttl: seconds(duration) })
  }
  setIdle(duration: Duration) {
    return this.lifetime({ idle: seconds(duration) })
  }
  metrics(options: { since?: string } = {}) {
    return this.#api.request<{
      at: string | null
      metrics: Record<string, unknown> | null
      samples?: { at: string; metrics: Record<string, unknown> }[]
    }>(this.path('/metrics') + (options.since ? `?${new URLSearchParams({ since: options.since })}` : ''))
  }
  async waitUntil(state: SandboxView['state'], timeout: Duration = SDK_DEFAULTS.runTimeoutSeconds) {
    const deadline = Date.now() + seconds(timeout) * SDK_DEFAULTS.secondMs
    do {
      await this.refresh()
      if (this.state === state) return this
      if (this.state === 'failed') throw new PryxError(0, 'sandbox_failed', 'Sandbox entered failed state')
      await new Promise((resolve) => setTimeout(resolve, SDK_DEFAULTS.secondMs / 10))
    } while (Date.now() < deadline)
    throw new PryxError(0, 'wait_timeout', `Sandbox did not reach ${state}`)
  }
  async *events(options: { since?: string; follow?: boolean; signal?: AbortSignal } | string = {}) {
    if (typeof options === 'string') options = { since: options }
    const query = new URLSearchParams({
      stream: 'true',
      follow: String(options.follow ?? false),
      ...(options.since ? { since: options.since } : {}),
    })
    yield* ndjson<{ id: string | number; kind: string; at: string; data: unknown }>(
      await this.#api.raw(this.path('/events') + `?${query}`, 'GET', undefined, undefined, options.signal),
    )
  }
}
