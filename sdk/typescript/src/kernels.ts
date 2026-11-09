// SPDX-License-Identifier: FSL-1.1-ALv2
/** Persistent language handles preserve all kernel MIME forms. Streaming never replays code;
 * collected results keep displays and tracebacks separate from stdout and stderr. */
import type { Agent } from './agent.ts'
import { type Duration, seconds } from './duration.ts'
import type { components } from './generated/agent.ts'
import { SDK_DEFAULTS } from './generated/defaults.ts'
import { KERNEL_DEFAULTS } from './generated/kernels.ts'
import { ndjson, PryxError } from './transport.ts'

export type KernelEvent = components['schemas']['KernelEvent']
export type KernelView = components['schemas']['KernelView']
export type KernelOptions = { context?: string }
export type KernelRunOptions = { timeout?: Duration }
export type KernelError = Omit<Extract<KernelEvent, { type: 'error' }>, 'type'>
type RichEvent = Extract<KernelEvent, { type: 'result' | 'display' }>

function checked(value: unknown): KernelEvent {
  if (!value || typeof value !== 'object') throw new PryxError(0, 'agent_protocol', 'Invalid kernel event')
  const event = value as Record<string, unknown>
  const count = (v: unknown) => v === null || (typeof v === 'number' && Number.isSafeInteger(v) && v >= 0)
  const object = (v: unknown) => v !== null && typeof v === 'object' && !Array.isArray(v)
  const valid =
    event.type === 'stream'
      ? ['stdout', 'stderr'].includes(String(event.stream)) && typeof event.text === 'string'
      : event.type === 'result' || event.type === 'display'
        ? object(event.data) &&
          object(event.metadata) &&
          (event.type === 'display' || count(event.executionCount))
        : event.type === 'error'
          ? typeof event.name === 'string' &&
            typeof event.value === 'string' &&
            Array.isArray(event.traceback) &&
            event.traceback.every((line) => typeof line === 'string')
          : event.type === 'done' && typeof event.status === 'string' && count(event.executionCount)
  if (!valid) throw new PryxError(0, 'agent_protocol', 'Invalid kernel event')
  return value as KernelEvent
}

export class KernelResult {
  readonly events: KernelEvent[] = []
  readonly results: RichEvent[] = []
  stdout = ''
  stderr = ''
  error?: KernelError
  executionCount: number | null = null
  status = ''
  private bytes = 0
  append(event: KernelEvent) {
    this.bytes += new TextEncoder().encode(JSON.stringify(event)).length
    if (this.bytes > SDK_DEFAULTS.captureBytes * 2)
      throw new PryxError(0, 'output_overflow', 'Kernel result exceeded capture budget')
    this.events.push(event)
    if (event.type === 'stream') {
      if (event.stream === 'stdout') this.stdout += event.text
      else this.stderr += event.text
    } else if (event.type === 'result' || event.type === 'display') this.results.push(event)
    else if (event.type === 'error')
      this.error = { name: event.name, value: event.value, traceback: event.traceback }
    else {
      this.executionCount = event.executionCount
      this.status = event.status
    }
  }
  private mime(name: string): unknown {
    for (let i = this.results.length - 1; i >= 0; i--) {
      const data = this.results[i]?.data as Record<string, unknown>
      if (Object.hasOwn(data, name)) return data[name]
    }
  }
  get text() {
    return this.mime('text/plain') as string | undefined
  }
  get html() {
    return this.mime('text/html') as string | undefined
  }
  get json() {
    return this.mime('application/json')
  }
  get svg() {
    return this.mime('image/svg+xml') as string | undefined
  }
  get png(): Uint8Array | undefined {
    const value = this.mime('image/png')
    if (value === undefined) return undefined
    if (typeof value !== 'string') throw new PryxError(0, 'agent_protocol', 'Invalid kernel PNG')
    try {
      return Uint8Array.from(atob(value), (c) => c.charCodeAt(0))
    } catch {
      throw new PryxError(0, 'agent_protocol', 'Invalid kernel PNG')
    }
  }
}

export class Kernel {
  private constructor(
    private agent: Agent,
    private view: KernelView,
  ) {}
  static async open(agent: Agent, language: KernelView['language'], options: KernelOptions = {}) {
    await agent.requireKernels(KERNEL_DEFAULTS.version)
    return new Kernel(
      agent,
      await agent.request<KernelView>('/_pryx/kernels', 'POST', { language, context: options.context ?? '' }),
    )
  }
  get id() {
    return this.view.id
  }
  get pid() {
    return this.view.pid
  }
  get processId() {
    return this.view.processId
  }
  get context() {
    return this.view.context
  }
  toJSON() {
    return this.view
  }
  private path(suffix = '') {
    return `/_pryx/kernels/${encodeURIComponent(this.id)}${suffix}`
  }
  stream(code: string, options: KernelRunOptions = {}): AsyncIterable<KernelEvent> {
    // An omitted timeout is the agent's to choose: its default, bounded by the token's ceiling.
    const started = this.agent.raw(
      this.path('/execute'),
      'POST',
      JSON.stringify(options.timeout === undefined ? { code } : { code, timeout: seconds(options.timeout) }),
      { 'content-type': 'application/json' },
    )
    void started.catch(() => {})
    let consumed = false
    return {
      async *[Symbol.asyncIterator]() {
        if (consumed) throw new PryxError(0, 'stream_consumed', 'Kernel streams have one consumer')
        consumed = true
        let done = false
        for await (const value of ndjson<unknown>(await started, SDK_DEFAULTS.captureBytes * 2)) {
          if (done) throw new PryxError(0, 'agent_protocol', 'Kernel sent data after done')
          const event = checked(value)
          done = event.type === 'done'
          yield event
        }
        if (!done)
          throw new PryxError(
            0,
            'kernel_interrupted',
            'Kernel stream ended without completion; inspect the kernel before retrying',
          )
      },
    }
  }
  async run(code: string, options: KernelRunOptions = {}): Promise<KernelResult> {
    const result = new KernelResult()
    for await (const event of this.stream(code, options)) result.append(event)
    return result
  }
  complete(code: string) {
    return this.agent.request<{ status: string; indent?: string }>(this.path('/complete'), 'POST', { code })
  }
  interrupt() {
    return this.agent.request<void>(this.path('/interrupt'), 'POST')
  }
  async restart() {
    this.view = await this.agent.request<KernelView>(this.path('/restart'), 'POST')
    return this
  }
  close() {
    return this.agent.request<void>(this.path(), 'DELETE')
  }
}
