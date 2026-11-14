// SPDX-License-Identifier: FSL-1.1-ALv2
/** Rivet's sandbox adapter over the public SDK. It owns no actor state and never retries mutations.
 * File paths resolve inside persistent /data; execution and preview routing remain pryx's.
 */
import { posix } from 'node:path'
import { CommandStream, Pryx, PryxError, type Sandbox, type SandboxOptions } from '@cubepals/pryx'
import type {
  Sandbox as AdapterSandbox,
  SandboxExecOptions,
  SandboxProvider,
} from '@rivet-dev/sandbox-adapter'
import { StatusCodes } from 'http-status-codes'
import { bytes } from './agent.ts'
import { SDK_DEFAULTS } from './generated/defaults.ts'

export type PryxProviderOptions = {
  url?: string
  key?: string
  tier?: SandboxOptions['isolation']
  template?: string
  require?: readonly string[]
}

const cwd = '/data'

function client(options: PryxProviderOptions) {
  const url = options.url ?? process.env.PRYX_API_URL
  const key = options.key ?? process.env.PRYX_API_KEY
  if (!url || !key) throw new Error('Set PRYX_API_URL and PRYX_API_KEY, or pass url and key')
  return new Pryx({ url, key })
}

function requireGuarantees(options: PryxProviderOptions) {
  if (options.require?.length)
    throw new PryxError(
      StatusCodes.UNPROCESSABLE_ENTITY,
      'missing_guarantee',
      `Required guarantees cannot be proved by this API version: ${options.require.join(', ')}`,
      { missing: [...options.require] },
    )
}

/** One provider can serve every actor; the actor persists the id returned by create. */
export function pryxProvider(options: PryxProviderOptions = {}): SandboxProvider {
  const api = client(options)
  return {
    name: 'pryx',
    cwd,
    async create() {
      requireGuarantees(options)
      const defaults = (await api.project()).defaults
      const tier = options.tier ?? defaults?.isolation
      return (
        await api.sandbox({
          isolation: options.tier,
          template: options.template,
          ...(!options.template && (tier === undefined || tier === 'container')
            ? { image: defaults?.repoImage }
            : {}),
        })
      ).id
    },
    async connect(_context, id) {
      requireGuarantees(options)
      let box: Sandbox
      try {
        box = await api.sandbox(id, { wake: false })
      } catch (error) {
        if (missing(error)) return undefined
        throw error
      }
      if (options.tier && box.isolation !== options.tier)
        throw new PryxError(
          StatusCodes.CONFLICT,
          'isolation_mismatch',
          `Required tier: ${options.tier}; sandbox tier: ${box.isolation}`,
        )
      if (['sleeping', 'resting'].includes(box.state)) {
        try {
          await box.wake()
        } catch (error) {
          if (missing(error)) return undefined
          throw error
        }
      }
      return adapterSandbox(box)
    },
    async suspend(_context, id) {
      let box: Sandbox
      try {
        box = await api.sandbox(id, { wake: false })
      } catch (error) {
        if (missing(error)) return
        throw error
      }
      await box.sleep()
    },
    async destroy(_context, id) {
      let box: Sandbox
      try {
        box = await api.sandbox(id, { wake: false })
      } catch (error) {
        if (missing(error)) return
        throw error
      }
      await box.delete()
    },
  }
}

/** Publish a sandbox port through pryx's edge, preserving its policy and capacity refusals. */
export async function pryxPortAddress(id: string, port: number, options: PryxProviderOptions = {}) {
  requireGuarantees(options)
  const box = await client(options).sandbox(id, { wake: false })
  if (options.tier && box.isolation !== options.tier)
    throw new PryxError(
      StatusCodes.CONFLICT,
      'isolation_mismatch',
      `Required tier: ${options.tier}; sandbox tier: ${box.isolation}`,
    )
  if (['sleeping', 'resting'].includes(box.state)) await box.wake()
  return box.expose(port)
}

function missing(error: unknown) {
  return (
    error instanceof PryxError &&
    (error.status === StatusCodes.NOT_FOUND || error.status === StatusCodes.GONE)
  )
}

function path(value: string) {
  if (!posix.isAbsolute(value) || value.includes('\0'))
    throw new TypeError('Sandbox file paths must be absolute paths inside /data')
  const normalized = posix.normalize(value)
  if (normalized !== cwd && !normalized.startsWith(`${cwd}/`))
    throw new TypeError(`Sandbox file path is outside persistent /data: ${value}`)
  return posix.relative(cwd, normalized) || '.'
}

function adapterSandbox(box: Sandbox): AdapterSandbox {
  return {
    cwd,
    exec: (command, options) => exec(box, command, options),
    readFile: (value) => box.files.readBytes(path(value)),
    writeFile: (value, content) => box.files.write(path(value), content),
    mkdir: (value) => box.files.mkdir(path(value)),
    stat: async (value) => ({ isDirectory: (await box.files.stat(path(value))).kind === 'directory' }),
    readdir: async (value) => (await box.files.list(path(value))).map((entry) => entry.name),
    exists: (value) => box.files.exists(path(value)),
  }
}

async function exec(box: Sandbox, command: string, options: SandboxExecOptions) {
  if (options.signal?.aborted) throw new Error('aborted')
  if (!posix.isAbsolute(options.cwd)) throw new TypeError('Exec cwd must be an absolute path')
  if (options.timeoutMs !== undefined && (!Number.isFinite(options.timeoutMs) || options.timeoutMs < 0))
    throw new TypeError('timeoutMs must be finite and nonnegative')
  const process = await box.spawn(command, { cwd: options.cwd, env: options.env })
  let killed: Promise<void> | undefined
  let timedOut = false
  let observing = true
  const observation = new AbortController()
  const killFailure = Promise.withResolvers<never>()
  void killFailure.promise.catch(() => {})
  const kill = () => {
    killed ??= process.kill()
    void killed.catch(killFailure.reject)
    return killed
  }
  const abort = () => {
    void kill()
  }
  options.signal?.addEventListener('abort', abort, { once: true })
  if (options.signal?.aborted) abort()
  let timer: ReturnType<typeof setTimeout> | undefined
  const deadline = performance.now() + (options.timeoutMs ?? 0)
  const checkDeadline = () => {
    const remaining = deadline - performance.now()
    if (remaining <= 0) {
      timedOut = true
      void kill()
    } else {
      // Bounded slices keep long caller deadlines within the JavaScript timer range.
      timer = setTimeout(
        checkDeadline,
        Math.min(remaining, SDK_DEFAULTS.runTimeoutSeconds * SDK_DEFAULTS.secondMs),
      )
    }
  }
  if (options.timeoutMs) checkDeadline()
  const stream = new CommandStream(
    Promise.resolve({
      async *events() {
        for await (const event of process.events(observation.signal)) {
          if (observing && event.type === 'output') options.onData?.(bytes(event.bytes))
          yield event
        }
      },
    }),
    false,
  )
  try {
    const result = await Promise.race([stream.result(), killFailure.promise])
    await killed
    if (options.signal?.aborted) throw new Error('aborted')
    if (result.stdoutTruncated || result.stderrTruncated)
      throw new PryxError(0, 'output_truncated', 'Command output exceeded the pryx capture limit')
    return {
      exitCode: timedOut || result.timedOut || result.killed ? null : result.code,
      timedOut: timedOut || result.timedOut,
      stdout: result.stdout,
      stderr: result.stderr,
    }
  } catch (error) {
    await kill()
    throw error
  } finally {
    observing = false
    observation.abort()
    clearTimeout(timer)
    options.signal?.removeEventListener('abort', abort)
  }
}
