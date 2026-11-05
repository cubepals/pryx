// SPDX-License-Identifier: FSL-1.1-ALv2

/** The direct sandbox transport caches a scoped capability, never a project key. Every mutation
 * is sent once. Processes and terminals use this transport; lifecycle remains on the public API. */
import { StatusCodes } from 'http-status-codes'
import { ndjson, type Pryx, PryxError } from './api.ts'
import { type Duration, seconds } from './duration.ts'
import type { components } from './generated/api.ts'
import { SDK_DEFAULTS } from './generated/defaults.ts'

export type AgentStart = components['schemas']['AgentStart']
export type AgentEvent = components['schemas']['AgentEvent']
export type AgentResult = components['schemas']['AgentResult']
export type AgentProcess = components['schemas']['AgentProcess']
export type AgentExposed = components['schemas']['AgentExposed']
export type SandboxToken = components['schemas']['SandboxToken']
export type StartOptions = Omit<Partial<AgentStart>, 'command' | 'timeout'> & {
  timeout?: Duration
  idempotencyKey?: string
}
export class Agent {
  private credential?: SandboxToken
  private renewing?: Promise<SandboxToken>
  private generation = 0
  constructor(
    readonly api: Pryx,
    readonly id: string,
  ) {}
  invalidate() {
    this.generation++
    this.credential = undefined
    this.renewing = undefined
  }
  async token() {
    if (
      this.credential &&
      this.credential.expiresAt > Date.now() / SDK_DEFAULTS.secondMs + SDK_DEFAULTS.tokenRenewBeforeSeconds
    )
      return this.credential
    if (!this.renewing) {
      const generation = this.generation
      const pending = this.api
        .request<SandboxToken>(`/v1/sandboxes/${encodeURIComponent(this.id)}/token`, 'POST')
        .then((credential) => {
          if (generation === this.generation) this.credential = credential
          return credential
        })
        .finally(() => {
          if (generation === this.generation) this.renewing = undefined
        })
      this.renewing = pending
    }
    return this.renewing
  }
  async raw(path: string, method = 'GET', body?: BodyInit, headers: HeadersInit = {}) {
    const token = await this.token()
    const response = await this.api.fetch(`${token.url.replace(/\/$/, '')}${path}`, {
      method,
      body,
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
  async start(command: string | string[], options: StartOptions = {}) {
    const { timeout, idempotencyKey, ...body } = options
    const process = await this.request<AgentProcess>(
      '/_pryx/processes',
      'POST',
      { ...body, command, ...(timeout === undefined ? {} : { timeout: seconds(timeout) }) },
      idempotencyKey ?? crypto.randomUUID(),
    )
    return new Process(this, process)
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
  async socket(path: string, attach?: (socket: WebSocket) => void) {
    const token = await this.token()
    const socket = new WebSocket(
      `${token.url
        .replace(/^http:/, 'ws:')
        .replace(/^https:/, 'wss:')
        .replace(/\/$/, '')}${path}`,
      ['pryx', `pryx-token.${token.token}`],
    )
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
      yield { type: 'exit', ...event.result } as const
    } else yield { type: 'stdout', data: '', truncated: true } as const
  }
}
export class Process {
  readonly stdin: { write: (data: string | Uint8Array) => Promise<void>; close: () => Promise<void> }
  constructor(
    readonly agent: Agent,
    readonly view: AgentProcess,
  ) {
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
  wait() {
    return this.agent.request<AgentResult>(this.path('/wait'))
  }
  signal(signal: string) {
    return this.agent.request<void>(this.path('/signal'), 'POST', { signal })
  }
  kill() {
    return this.signal('KILL')
  }
  resize(cols: number, rows: number) {
    return this.agent.request<void>(this.path('/resize'), 'POST', { cols, rows })
  }
  async *events() {
    yield* ndjson<AgentEvent>(await this.agent.raw(this.path('/output')))
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
