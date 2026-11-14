// SPDX-License-Identifier: FSL-1.1-ALv2
/** Eager command observation with bounded capture and one live consumer; execution belongs to the agent. */
import { output, type Process } from './agent.ts'
import type { Duration } from './duration.ts'
import { SDK_DEFAULTS } from './generated/defaults.ts'
import { PryxError } from './transport.ts'

export type RunOptions = {
  timeout?: Duration
  env?: Record<string, string>
  cwd?: string
  stdin?: string | Uint8Array
  idempotencyKey?: string
}
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

export async function* lines(chunks: AsyncIterable<Chunk>): AsyncGenerator<string> {
  const rest = { stdout: '', stderr: '' }
  for await (const chunk of chunks) {
    if (rest[chunk.stream].length + chunk.data.length > SDK_DEFAULTS.captureBytes)
      throw new PryxError(0, 'line_too_long', 'Output line exceeds the bounded line buffer')
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

export class CommandStream implements AsyncIterable<Chunk> {
  #pending: Chunk[] = []
  #held = 0
  #done = false
  #consuming = false
  #failure?: unknown
  #streamFailure?: PryxError
  #wake?: () => void
  #result: Promise<RunResult>
  constructor(process: Promise<Pick<Process, 'events'>>, observe = true) {
    this.#result = (async () => {
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
        for await (const event of output((await process).events())) {
          if (event.type === 'exit') {
            Object.assign(result, {
              code: event.code,
              timedOut: event.timedOut,
              killed: event.killed,
              durationMs: event.durationMs,
            })
            exited = true
          } else {
            const room = Math.max(0, SDK_DEFAULTS.captureBytes - result[event.type].length)
            result[event.type] += event.data.slice(0, room)
            result[`${event.type}Truncated`] ||= event.truncated || event.data.length > room
            if (observe && !this.#streamFailure) {
              if (
                this.#held + event.data.length > SDK_DEFAULTS.captureBytes ||
                this.#pending.length >= 1024
              ) {
                this.#streamFailure = new PryxError(
                  0,
                  'stream_overflow',
                  'Command output consumer fell behind; result remains available',
                )
                this.#pending.length = 0
                this.#held = 0
              } else {
                this.#pending.push({ stream: event.type, data: event.data })
                this.#held += event.data.length
              }
              this.#wake?.()
            }
          }
        }
        if (!exited)
          throw new PryxError(
            0,
            'exec_interrupted',
            'Command ended without an exit result; its outcome is unknown',
          )
        return result
      } catch (error) {
        this.#failure = error
        throw error
      } finally {
        this.#done = true
        this.#wake?.()
      }
    })()
    void this.#result.catch(() => {})
  }
  result() {
    return this.#result
  }
  lines() {
    return lines(this)
  }
  async *[Symbol.asyncIterator](): AsyncGenerator<Chunk> {
    if (this.#consuming) throw new Error('A command stream has one consumer; use result() for its result')
    this.#consuming = true
    try {
      while (!this.#done || this.#pending.length) {
        if (this.#streamFailure) throw this.#streamFailure
        const chunk = this.#pending.shift()
        if (chunk) {
          this.#held -= chunk.data.length
          yield chunk
        } else
          await new Promise<void>((resolve) => {
            this.#wake = resolve
          })
      }
      if (this.#streamFailure) throw this.#streamFailure
      if (this.#failure) throw this.#failure
    } finally {
      this.#pending.length = 0
      this.#held = 0
      this.#streamFailure ??= new PryxError(0, 'stream_closed', 'Command observation closed')
    }
  }
}
