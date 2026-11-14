// SPDX-License-Identifier: FSL-1.1-ALv2

/** The direct sandbox transport caches a scoped capability, never a project key. Every mutation
 * is sent once. Processes and terminals use this transport; lifecycle remains on the public API. */
import { StatusCodes } from 'http-status-codes'
import { type Duration, seconds } from './duration.ts'
import type { components } from './generated/api.ts'
import { SDK_DEFAULTS } from './generated/defaults.ts'
import { type LogsOptions, logs } from './logs.ts'
import { ndjson, PryxError, type Transport } from './transport.ts'

export type AgentStart = components['schemas']['AgentStart']
export type AgentEvent = components['schemas']['AgentEvent']
export type AgentResult = components['schemas']['AgentResult']
export type AgentProcess = components['schemas']['AgentProcess']
export type AgentExposed = components['schemas']['AgentExposed']
export type SandboxToken = components['schemas']['SandboxToken']
export type StartOptions = Omit<Partial<AgentStart>, 'command' | 'timeout' | 'stdin'> & {
  timeout?: Duration
  stdin?: boolean | string | Uint8Array
  idempotencyKey?: string
}
export class Agent {
  #credential?: SandboxToken
  #renewing?: Promise<SandboxToken>
  #generation = 0
  #version?: Promise<void>
  #kernels?: Promise<void>
  #tools?: Promise<void>
  constructor(
    readonly api: Transport,
    readonly id: string,
  ) {}
  invalidate() {
    this.#version = undefined
    this.#tools = undefined
    this.#kernels = undefined
    this.#generation++
    this.#credential = undefined
    this.#renewing = undefined
  }
  async token() {
    if (
      this.#credential &&
      this.#credential.expiresAt > Date.now() / SDK_DEFAULTS.secondMs + SDK_DEFAULTS.tokenRenewBeforeSeconds
    )
      return this.#credential
    if (!this.#renewing) {
      const generation = this.#generation
      const pending = this.api
        .request<SandboxToken>(`/v1/sandboxes/${encodeURIComponent(this.id)}/token`, 'POST')
        .then((credential) => {
          if (generation === this.#generation) this.#credential = credential
          return credential
        })
        .finally(() => {
          if (generation === this.#generation) this.#renewing = undefined
        })
      this.#renewing = pending
    }
    return this.#renewing
  }
  async raw(path: string, method = 'GET', body?: BodyInit, headers: HeadersInit = {}, signal?: AbortSignal) {
    const token = await this.token()
    const response = await this.api.fetch(`${token.url.replace(/\/$/, '')}${path}`, {
      method,
      redirect: 'manual',
      signal,
      body,
      ...(body instanceof ReadableStream ? { duplex: 'half' } : {}),
      headers: { ...Object.fromEntries(new Headers(headers)), authorization: `Bearer ${token.token}` },
    })
    if (!response.ok) {
      const value = (await response.json().catch(() => ({}))) as {
        error?: { code?: string; message?: string; details?: unknown }
      }
      throw new PryxError(
        response.status,
        value.error?.code ?? 'agent_error',
        value.error?.message ?? response.statusText,
        value.error?.details,
      )
    }
    return response
  }
  async request<T>(path: string, method = 'GET', body?: unknown, key?: string): Promise<T> {
    const response = await this.raw(path, method, body === undefined ? undefined : JSON.stringify(body), {
      ...(body === undefined ? {} : { 'content-type': 'application/json' }),
      ...(key ? { 'idempotency-key': key } : {}),
    })
    return response.status === StatusCodes.NO_CONTENT ? (undefined as T) : (response.json() as Promise<T>)
  }
  requireVersion() {
    this.#version ??= this.request<{ apiVersion?: number }>('/_pryx/health')
      .then((health) => {
        if ((health.apiVersion ?? 1) < SDK_DEFAULTS.agentVersion)
          throw new PryxError(0, 'unsupported', 'This operation requires agent API version 2')
      })
      .catch((error) => {
        this.#version = undefined
        throw error
      })
    return this.#version
  }
  requireKernels(version: number) {
    this.#kernels ??= this.request<{ kernelsVersion?: number }>('/_pryx/health')
      .then((health) => {
        if ((health.kernelsVersion ?? 0) < version)
          throw new PryxError(0, 'unsupported', 'This agent does not provide language kernels')
      })
      .catch((error) => {
        this.#kernels = undefined
        throw error
      })
    return this.#kernels
  }
  requireTools(version: number) {
    this.#tools ??= this.request<{ toolsVersion?: number }>('/_pryx/health')
      .then((health) => {
        if ((health.toolsVersion ?? 0) < version)
          throw new PryxError(0, 'unsupported', 'This agent does not provide native tools')
      })
      .catch((error) => {
        this.#tools = undefined
        throw error
      })
    return this.#tools
  }
  async requestV2<T>(path: string, method = 'GET', body?: unknown): Promise<T> {
    await this.requireVersion()
    return this.request<T>(path, method, body)
  }
  logs(options: LogsOptions = {}, signal?: AbortSignal) {
    return logs(
      () =>
        this.raw(
          `/_pryx/logs?${new URLSearchParams({ follow: String(options.follow ?? false) })}`,
          'GET',
          undefined,
          {},
          signal,
        ),
      options,
    )
  }
  async start(command: string | string[], options: StartOptions = {}) {
    const { timeout, idempotencyKey, stdin, ...body } = options
    const value = timeout === undefined ? undefined : seconds(timeout)
    if (value !== undefined && !Number.isInteger(value))
      throw new TypeError('Process timeout is a whole number of seconds')
    const process = await this.request<AgentProcess>(
      '/_pryx/processes',
      'POST',
      {
        ...body,
        command,
        ...(stdin === undefined ? {} : { stdin: typeof stdin === 'boolean' ? stdin : true }),
        ...(value === undefined ? {} : { timeout: value }),
      },
      idempotencyKey ?? crypto.randomUUID(),
    )
    const handle = new Process(this, process)
    if (typeof stdin === 'string' || stdin instanceof Uint8Array) {
      const bytes = typeof stdin === 'string' ? new TextEncoder().encode(stdin) : stdin
      try {
        for (let offset = 0; offset < bytes.byteLength; offset += SDK_DEFAULTS.stdinBytes)
          await handle.stdin.write(bytes.subarray(offset, offset + SDK_DEFAULTS.stdinBytes))
        await handle.stdin.close()
      } catch (error) {
        throw new PryxError(
          0,
          'stdin_failed',
          'Input delivery failed; inspect the process before deciding whether to stop it',
          { processId: process.id, cause: error instanceof Error ? error.message : String(error) },
        )
      }
    }
    return handle
  }
  async processes() {
    return (await this.request<{ processes: AgentProcess[] }>('/_pryx/processes')).processes.map(
      (view) => new Process(this, view),
    )
  }
  async process(id: string) {
    return new Process(this, await this.request<AgentProcess>(`/_pryx/processes/${encodeURIComponent(id)}`))
  }
  expose(port: number) {
    return this.request<AgentExposed>('/_pryx/ports', 'POST', { port })
  }
  unexpose(port: number) {
    return this.request<void>(`/_pryx/ports/${port}`, 'DELETE')
  }
  ports() {
    return this.request<{ ports: AgentExposed[] }>('/_pryx/ports')
  }
  async terminal(options: { cols?: number; rows?: number } = {}) {
    let terminal: Terminal | undefined
    await this.socket('/_pryx/terminal', (socket) => {
      terminal = new Terminal(socket)
    })
    if (!terminal) throw new PryxError(0, 'agent_socket', 'Terminal did not open')
    if (options.cols !== undefined || options.rows !== undefined)
      terminal.resize(options.cols ?? 80, options.rows ?? 24)
    return terminal
  }
  async connection(path: string) {
    const token = await this.token()
    return {
      url: `${token.url
        .replace(/^http:/, 'ws:')
        .replace(/^https:/, 'wss:')
        .replace(/\/$/, '')}${path}`,
      protocols: ['pryx', `pryx-token.${token.token}`],
    }
  }
  async socket(path: string, attach?: (socket: WebSocket) => void) {
    const connection = await this.connection(path)
    const socket = new WebSocket(connection.url, connection.protocols)
    socket.binaryType = 'arraybuffer'
    attach?.(socket)
    await new Promise<void>((resolve, reject) => {
      socket.addEventListener('open', () => resolve(), { once: true })
      socket.addEventListener(
        'error',
        () => reject(new PryxError(0, 'agent_socket', 'Agent websocket failed')),
        { once: true },
      )
    })
    return socket
  }
}

export function binaryBody(data: string | Uint8Array): string | Uint8Array<ArrayBuffer> {
  if (typeof data === 'string') return data
  const buffer = data.buffer
  if (!(buffer instanceof ArrayBuffer)) throw new TypeError('Binary input requires ArrayBuffer storage')
  return new Uint8Array(buffer, data.byteOffset, data.byteLength)
}
function bytes(encoded: string) {
  return Uint8Array.from(atob(encoded), (c) => c.charCodeAt(0))
}
function encoded(data: string | Uint8Array) {
  const value = typeof data === 'string' ? new TextEncoder().encode(data) : data
  // Bounded pieces avoid a spread/call-stack limit on large stdin writes.
  let text = ''
  for (let at = 0; at < value.length; at += 8192)
    text += String.fromCharCode(...value.subarray(at, at + 8192))
  return btoa(text)
}
export async function* output(events: AsyncIterable<AgentEvent>) {
  const decoders = { stdout: new TextDecoder(), stderr: new TextDecoder() }
  for await (const event of events) {
    if (event.type === 'output') {
      if (event.stream !== 'stdout' && event.stream !== 'stderr')
        throw new PryxError(0, 'agent_protocol', 'Unknown output stream')
      const data = decoders[event.stream].decode(bytes(event.bytes), { stream: true })
      if (data) yield { type: event.stream, data, truncated: false } as const
    } else if (event.type === 'exit') {
      for (const stream of ['stdout', 'stderr'] as const) {
        const data = decoders[stream].decode()
        if (data) yield { type: stream, data, truncated: false } as const
      }
      if (event.result.stdoutTruncated) yield { type: 'stdout', data: '', truncated: true } as const
      if (event.result.stderrTruncated) yield { type: 'stderr', data: '', truncated: true } as const
      yield { type: 'exit', ...event.result } as const
    } else yield { type: 'stdout', data: '', truncated: true } as const
  }
}
export class Process {
  #agent: Agent
  readonly stdin: { write: (data: string | Uint8Array) => Promise<void>; close: () => Promise<void> }
  constructor(
    agent: Agent,
    readonly view: AgentProcess,
  ) {
    this.#agent = agent
    this.stdin = {
      write: async (data) => {
        await agent.raw(this.path('/stdin'), 'POST', binaryBody(data))
      },
      close: () => agent.request(this.path('/stdin'), 'DELETE'),
    }
  }
  get id() {
    return this.view.id
  }
  get pid() {
    return this.view.pid
  }
  private path(suffix = '') {
    return `/_pryx/processes/${encodeURIComponent(this.id)}${suffix}`
  }
  waitForPort(
    port: number,
    options: { mode?: 'http' | 'tcp'; path?: string; status?: number; timeout?: Duration } = {},
  ) {
    return this.#agent.requestV2<{ ready: true }>(this.path('/ready'), 'POST', {
      kind: 'port',
      port,
      ...options,
      timeout: seconds(options.timeout ?? SDK_DEFAULTS.runTimeoutSeconds),
    })
  }
  waitForLog(pattern: string | RegExp, timeout: Duration = SDK_DEFAULTS.runTimeoutSeconds) {
    if (pattern instanceof RegExp && pattern.flags)
      throw new TypeError('Pass an unflagged regular expression or use inline regex flags')
    return this.#agent.requestV2<{ matched: true }>(this.path('/ready'), 'POST', {
      kind: 'log',
      pattern: typeof pattern === 'string' ? pattern : pattern.source,
      timeout: seconds(timeout),
    })
  }
  waitForExit(timeout: Duration = SDK_DEFAULTS.runTimeoutSeconds) {
    return this.#agent.requestV2<AgentResult>(this.path('/ready'), 'POST', {
      kind: 'exit',
      timeout: seconds(timeout),
    })
  }
  logs() {
    return this.stream()
  }
  wait() {
    return this.#agent.request<AgentResult>(this.path('/wait'))
  }
  signal(signal: string) {
    return this.#agent.request<void>(this.path('/signal'), 'POST', { signal })
  }
  kill() {
    return this.signal('KILL')
  }
  resize(cols: number, rows: number) {
    return this.#agent.request<void>(this.path('/resize'), 'POST', { cols, rows })
  }
  async *events() {
    yield* ndjson<AgentEvent>(await this.#agent.raw(this.path('/output')))
  }
  async *stream() {
    for await (const event of output(this.events()))
      if (event.type === 'stdout' || event.type === 'stderr') yield { stream: event.type, data: event.data }
  }
  async *lines() {
    const rest = { stdout: '', stderr: '' }
    for await (const chunk of this.stream()) {
      if (rest[chunk.stream].length + chunk.data.length > SDK_DEFAULTS.captureBytes)
        throw new PryxError(0, 'line_too_long', 'Output line exceeds the bounded line buffer; use stream()')
      rest[chunk.stream] += chunk.data
      let at = rest[chunk.stream].indexOf('\n')
      while (at >= 0) {
        yield rest[chunk.stream].slice(0, at).replace(/\r$/, '')
        rest[chunk.stream] = rest[chunk.stream].slice(at + 1)
        at = rest[chunk.stream].indexOf('\n')
      }
    }
    for (const value of Object.values(rest)) if (value) yield value
  }
}
export class Terminal {
  private pending: AgentEvent[] = []
  private wake?: () => void
  private ended = false
  private failure?: Error
  readonly stdin: { write: (data: string | Uint8Array) => void }
  constructor(readonly socket: WebSocket) {
    this.stdin = { write: (data) => socket.send(JSON.stringify({ type: 'stdin', bytes: encoded(data) })) }
    socket.addEventListener('message', (event) => {
      try {
        this.pending.push(JSON.parse(String(event.data)) as AgentEvent)
      } catch {
        this.failure = new PryxError(0, 'agent_protocol', 'Invalid terminal frame')
        socket.close()
        this.ended = true
      }
      if (this.pending.length > 128) {
        this.failure = new PryxError(0, 'agent_backpressure', 'Terminal consumer fell behind')
        socket.close()
        this.ended = true
      }
      this.wake?.()
    })
    socket.addEventListener('close', () => {
      this.ended = true
      this.wake?.()
    })
  }
  resize(cols: number, rows: number) {
    this.socket.send(JSON.stringify({ type: 'resize', cols, rows }))
  }
  signal(signal: string) {
    this.socket.send(JSON.stringify({ type: 'signal', signal }))
  }
  close() {
    this.socket.close()
  }
  async *events() {
    while (!this.ended || this.pending.length) {
      if (this.failure) throw this.failure
      const event = this.pending.shift()
      if (event) yield event
      else
        await new Promise<void>((resolve) => {
          this.wake = resolve
        })
    }
    if (this.failure) throw this.failure
  }
  async *stream() {
    for await (const event of output(this.events()))
      if (event.type === 'stdout' || event.type === 'stderr') yield { stream: event.type, data: event.data }
  }
}
