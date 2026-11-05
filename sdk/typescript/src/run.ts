// SPDX-License-Identifier: FSL-1.1-ALv2
/** One lazy command: awaiting collects its result; streaming consumes it once with backpressure. No command is retried or replayed. */
import { type ExecRequest, type Pryx, PryxError } from './api.ts'
import { type Duration, seconds } from './duration.ts'
export type RunOptions = { timeout?: Duration; env?: Record<string, string>; idempotencyKey?: string }
export type RunResult = {
  stdout: string
  stderr: string
  code: number | null
  timedOut: boolean
  killed: boolean
  durationMs: number
  stdoutTruncated: boolean
  stderrTruncated: boolean
}
export type Chunk = { stream: 'stdout' | 'stderr'; data: string }
export class Run implements PromiseLike<RunResult> {
  private result?: Promise<RunResult>
  private resolve?: (result: RunResult) => void
  private reject?: (error: unknown) => void
  private consuming = false
  readonly key: string
  readonly body: ExecRequest
  constructor(
    private api: Pryx,
    private id: string,
    command: string | string[],
    options: RunOptions = {},
  ) {
    this.key = options.idempotencyKey ?? crypto.randomUUID()
    this.body = {
      command: typeof command === 'string' ? ['sh', '-c', command] : command,
      ...(options.timeout === undefined ? {} : { timeout: seconds(options.timeout) }),
      ...(options.env ? { env: options.env } : {}),
    }
    if (this.body.timeout !== undefined && !Number.isInteger(this.body.timeout))
      throw new TypeError('Exec timeout is a whole number of seconds')
  }
  private completion() {
    if (!this.result) {
      this.result = new Promise<RunResult>((resolve, reject) => {
        this.resolve = resolve
        this.reject = reject
      })
      // A stream-only caller observes errors through its iterator.
      void this.result.catch(() => {})
    }
    return this.result
  }
  // biome-ignore lint/suspicious/noThenProperty: The public run value deliberately implements PromiseLike.
  then<A = RunResult, B = never>(
    fulfilled?: ((value: RunResult) => A | PromiseLike<A>) | null,
    rejected?: ((reason: unknown) => B | PromiseLike<B>) | null,
  ): Promise<A | B> {
    const completion = this.completion()
    if (!this.consuming)
      void (async () => {
        try {
          for await (const _ of this.stream()) {
          }
        } catch {}
      })()
    return completion.then(fulfilled, rejected)
  }
  async *stream(): AsyncGenerator<Chunk> {
    if (this.consuming) throw new Error('A run has one stream consumer; await the same run for its result')
    this.consuming = true
    this.completion()
    const result: RunResult = {
      stdout: '',
      stderr: '',
      code: null,
      timedOut: false,
      killed: false,
      durationMs: 0,
      stdoutTruncated: false,
      stderrTruncated: false,
    }
    let exited = false
    try {
      for await (const event of this.api.exec(this.id, this.body, this.key)) {
        if (event.type === 'error')
          throw new PryxError(0, event.error.code, event.error.message, event.error.details)
        if (event.type === 'stdout' || event.type === 'stderr') {
          result[event.type] += event.data
          if (event.type === 'stdout') result.stdoutTruncated ||= event.truncated
          else result.stderrTruncated ||= event.truncated
          yield { stream: event.type, data: event.data }
        } else if (event.type === 'exit') {
          Object.assign(result, {
            code: event.code,
            timedOut: event.timedOut,
            killed: event.killed,
            durationMs: event.durationMs,
          })
          exited = true
        }
      }
      if (!exited)
        throw new PryxError(
          0,
          'exec_interrupted',
          'Exec ended without an exit result; its outcome is unknown',
        )
      this.resolve?.(result)
    } catch (error) {
      this.reject?.(error)
      throw error
    } finally {
      if (!exited)
        this.reject?.(new PryxError(0, 'exec_interrupted', 'Exec stream was closed; its outcome is unknown'))
    }
  }
  async *lines(): AsyncGenerator<string> {
    const rest = { stdout: '', stderr: '' }
    for await (const chunk of this.stream()) {
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
