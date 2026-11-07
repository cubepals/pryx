// SPDX-License-Identifier: FSL-1.1-ALv2
/** Git commands use fixed argv and one-call credential helpers. Secrets never enter a URL,
 * argv, file or repository config; parsed results retain their bounded raw process result. */

import { SDK_DEFAULTS } from './generated/defaults.ts'
import { type GitSpawn, gitProxy } from './git-proxy.ts'
import type { RunOptions, RunResult } from './run.ts'
import { PryxError } from './transport.ts'

type GitOptions = { timeout?: RunOptions['timeout'] }
type Execute = (command: string[], options?: RunOptions) => Promise<RunResult>
export type GitResult<T> = { value: T; raw: RunResult }
export class Git {
  #path = 'repo'
  constructor(
    private execute: Execute,
    private spawn?: GitSpawn,
    private isolation?: string,
  ) {}
  private async call(
    args: string[],
    options: {
      token?: string
      path?: string
      env?: Record<string, string>
      root?: boolean
      timeout?: RunOptions['timeout']
      credentialUrl?: string
    } = {},
  ) {
    const { token, path, env } = options
    const timeout =
      options.timeout ??
      (['clone', 'push', 'pull'].includes(args[0] ?? '')
        ? SDK_DEFAULTS.gitTimeoutSeconds
        : SDK_DEFAULTS.runTimeoutSeconds)
    let credentialHost = ''
    if (token) {
      if (/[\r\n\0]/.test(token)) throw new TypeError('Git token must be a single line')
      const remote =
        options.credentialUrl ??
        (await this.call(['remote', 'get-url', 'origin'], { timeout: options.timeout })).stdout.trim()
      const url = new URL(remote)
      if (url.protocol !== 'https:' || url.username || url.password || url.search || url.hash)
        throw new TypeError('Git tokens require an HTTPS remote without credentials or query')
      credentialHost = url.host
    }
    let bridge: Awaited<ReturnType<typeof gitProxy>> | undefined
    if (this.isolation === 'wasm' && this.spawn && ['clone', 'push', 'pull'].includes(args[0] ?? '')) {
      const remote =
        options.credentialUrl ??
        (await this.call(['remote', 'get-url', 'origin'], { timeout: options.timeout })).stdout.trim()
      if (/^https?:/.test(remote)) bridge = await gitProxy(this.spawn, remote, timeout)
    }
    if (bridge && token) credentialHost = bridge.host
    try {
      const raw = await this.execute(
        [
          'env',
          '-u',
          'GIT_CURL_VERBOSE',
          '-u',
          'GIT_TRACE_CURL',
          '-u',
          'GIT_TRACE',
          '-u',
          'GIT_TRACE_PACKET',
          '-u',
          'GIT_TRACE_PACK_ACCESS',
          '-u',
          'GIT_TRACE_SETUP',
          '-u',
          'GIT_TRACE_PERFORMANCE',
          '-u',
          'GIT_TRACE2',
          '-u',
          'GIT_TRACE2_EVENT',
          '-u',
          'GIT_TRACE2_PERF',
          '-u',
          'GIT_TRACE2_ENV_VARS',
          '-u',
          'GIT_TRACE2_CONFIG_PARAMS',
          '-u',
          'GIT_TRACE2_BRIEF',
          '-u',
          'GIT_TRACE2_EVENT_BRIEF',
          '-u',
          'GIT_CONFIG_PARAMETERS',
          '-u',
          'GIT_SSL_NO_VERIFY',
          'git',
          ...(bridge?.args ?? []),
          '-c',
          'credential.helper=',
          '-c',
          'http.followRedirects=false',
          '-c',
          'core.hooksPath=/dev/null',
          '-c',
          'core.fsmonitor=false',
          '-c',
          'maintenance.auto=false',
          '-c',
          'gc.auto=0',
          ...(options.root ? [] : ['-C', this.#path]),
          ...(token
            ? [
                '-c',
                'credential.helper=!f() { [ "$1" = get ] || return 0; protocol=; host=; while IFS="=" read -r key value; do case "$key" in protocol) protocol=$value;; host) host=$value;; esac; done; [ "$protocol" = "$PRYX_GIT_PROTOCOL" ] && [ "$host" = "$PRYX_GIT_HOST" ] || return 1; printf "%s\\n" "username=x-access-token" "password=$PRYX_GIT_TOKEN"; }; f',
              ]
            : []),
          ...args,
        ],
        {
          timeout,
          ...(path ? { cwd: path } : {}),
          env: {
            GIT_TERMINAL_PROMPT: '0',
            GIT_CONFIG_COUNT: '0',
            GIT_CONFIG_NOSYSTEM: '1',
            GIT_CONFIG_GLOBAL: '/dev/null',
            ...env,
            PRYX_GIT_PROTOCOL: bridge?.protocol ?? 'https',
            PRYX_GIT_TOKEN: token ?? '',
            PRYX_GIT_HOST: credentialHost,
          },
        },
      )
      if (token) {
        raw.stdout = raw.stdout.replaceAll(token, '[redacted]')
        raw.stderr = raw.stderr.replaceAll(token, '[redacted]')
      }
      if (raw.stdoutTruncated)
        throw new PryxError(
          0,
          'git_output_truncated',
          'Git stdout exceeded the capture budget; parsed output is incomplete',
        )
      if (raw.code !== 0)
        throw new PryxError(0, 'git_failed', `git ${args[0]} failed: ${raw.stderr}`, { raw })
      return raw
    } finally {
      await bridge?.close()
    }
  }
  async clone(
    url: string,
    options: GitOptions & { path?: string; branch?: string; depth?: number; token?: string } = {},
  ) {
    if (options.token && (!/^https:\/\//.test(url) || /[\r\n\0]/.test(options.token)))
      throw new TypeError('Git tokens require HTTPS and a single line')
    if (options.depth !== undefined && (!Number.isSafeInteger(options.depth) || options.depth <= 0))
      throw new TypeError('depth must be a positive integer')
    if (/^https?:/.test(url)) {
      const remote = new URL(url)
      if (remote.username || remote.password || remote.search || remote.hash)
        throw new TypeError('Git URLs cannot contain credentials, queries or fragments')
    }
    const path = options.path ?? 'repo'
    const raw = await this.call(
      [
        'clone',
        ...(this.isolation === 'wasm' ? ['--separate-git-dir=/tmp/.pryx-git-' + crypto.randomUUID()] : []),
        ...(options.branch ? ['--branch', options.branch] : []),
        ...(options.depth ? ['--depth', String(options.depth)] : []),
        '--',
        url,
        path,
      ],
      { token: options.token, timeout: options.timeout, root: true, credentialUrl: url },
    )
    this.#path = path
    return { value: { path }, raw }
  }
  async status(
    options: GitOptions = {},
  ): Promise<GitResult<{ clean: boolean; entries: { status: string; path: string }[] }>> {
    const raw = await this.call(['status', '--porcelain=v1', '-z'], options)
    const fields = raw.stdout.split('\0')
    const entries = []
    for (let i = 0; i < fields.length; i++) {
      const field = fields[i]
      if (!field) continue
      entries.push({ status: field.slice(0, 2), path: field.slice(3) })
      if (/[RC]/.test(field.slice(0, 2))) i++
    }
    return { value: { clean: entries.length === 0, entries }, raw }
  }
  async diff(options: GitOptions & { staged?: boolean } = {}) {
    const raw = await this.call(
      ['diff', ...(options.staged ? ['--cached'] : []), '--no-ext-diff', '--no-textconv'],
      options,
    )
    return { value: { patch: raw.stdout }, raw }
  }
  async branch(name?: string, options: GitOptions = {}) {
    if (name?.startsWith('-')) throw new TypeError('Invalid branch name')
    const raw = await this.call(
      name ? ['branch', '--', name] : ['branch', '--format=%(refname:short)'],
      options,
    )
    return { value: raw.stdout.trim().split('\n').filter(Boolean), raw }
  }
  async checkout(ref: string, options: GitOptions & { create?: boolean } = {}) {
    if (ref.startsWith('-')) throw new TypeError('Invalid ref')
    const raw = await this.call(['checkout', ...(options.create ? ['-b'] : []), ref, '--'], options)
    return { value: { ref }, raw }
  }
  async add(paths: string | string[], options: GitOptions = {}) {
    const raw = await this.call(['add', '--', ...(typeof paths === 'string' ? [paths] : paths)], options)
    return { value: { added: true }, raw }
  }
  async commit(message: string, options: GitOptions & { author?: { name: string; email: string } } = {}) {
    const env: Record<string, string> = options.author
      ? {
          GIT_AUTHOR_NAME: options.author.name,
          GIT_AUTHOR_EMAIL: options.author.email,
          GIT_COMMITTER_NAME: options.author.name,
          GIT_COMMITTER_EMAIL: options.author.email,
        }
      : {}
    const raw = await this.call(['commit', '--no-gpg-sign', '-m', message], { env, timeout: options.timeout })
    const head = await this.call(['rev-parse', 'HEAD'], { timeout: options.timeout })
    return { value: { hash: head.stdout.trim() }, raw }
  }
  async push(options: GitOptions & { token?: string } = {}) {
    const raw = await this.call(['push'], options)
    return { value: { pushed: true }, raw }
  }
  async pull(options: GitOptions = {}) {
    const raw = await this.call(['pull', '--ff-only'], options)
    return { value: { pulled: true }, raw }
  }
  async log(options: GitOptions & { limit?: number } = {}) {
    const limit = options.limit ?? 10
    if (!Number.isSafeInteger(limit) || limit <= 0) throw new TypeError('limit must be a positive integer')
    const raw = await this.call(['log', `-${limit}`, '--format=%H%x00%an%x00%ae%x00%aI%x00%s%x00'], options)
    const fields = raw.stdout.split('\0')
    const value = []
    for (let i = 0; i + 4 < fields.length; i += 5)
      value.push({
        hash: fields[i]?.trim(),
        author: { name: fields[i + 1], email: fields[i + 2] },
        at: fields[i + 3],
        subject: fields[i + 4],
      })
    return { value, raw }
  }
}
