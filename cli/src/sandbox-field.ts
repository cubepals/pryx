// SPDX-License-Identifier: FSL-1.1-ALv2
/** Sandbox convenience commands use the same SDK methods as applications. Interactive shell
 * commands retain state through a pipe shell; the terminal command remains a full PTY. */

import { createInterface } from 'node:readline'
import type { Duration, Sandbox } from '@cubepals/pryx'
import { seconds } from '../../sdk/typescript/src/duration.ts'
import { SDK_DEFAULTS } from '../../sdk/typescript/src/generated/defaults.ts'

function option(args: string[], name: string) {
  const at = args.indexOf(name)
  if (at < 0) return undefined
  const value = args[at + 1]
  if (value === undefined) throw new Error(`${name} needs a value`)
  return value
}
function duration(text: string) {
  return /^\d+(?:\.\d+)?$/.test(text) ? seconds(Number(text)) : seconds(text as Duration)
}
export async function field(
  box: Sandbox,
  verb: string,
  args: string[],
): Promise<{ handled: boolean; result?: unknown }> {
  const first = args[0]
  let result: unknown
  switch (verb) {
    case 'ps':
      result = (await box.processes()).map((process) => process.view)
      break
    case 'kill-all':
      result = await box.killAll()
      break
    case 'spawn': {
      const split = args.indexOf('--')
      const command = split < 0 ? args : args.slice(split + 1)
      result = (
        await box.spawn(command, {
          ...(args.slice(0, split < 0 ? args.length : split).includes('--stdin')
            ? { stdin: new Uint8Array(await Bun.stdin.arrayBuffer()) }
            : {}),
        })
      ).view
      break
    }
    case 'wait-port':
    case 'wait-log':
    case 'wait-exit': {
      const process = (await box.processes()).find((process) => process.id === first)
      if (!process) throw new Error('No such process')
      const timeout = option(args, '--timeout')
        ? duration(option(args, '--timeout') as string)
        : SDK_DEFAULTS.runTimeoutSeconds
      result =
        verb === 'wait-port'
          ? await process.waitForPort(Number(args[1]), {
              mode: args.includes('--http') ? 'http' : 'tcp',
              path: option(args, '--path'),
              timeout,
            })
          : verb === 'wait-log'
            ? await process.waitForLog(args[1] ?? '', timeout)
            : await process.waitForExit(timeout)
      break
    }
    case 'shell': {
      const shell = await box.shell({ cwd: option(args, '--cwd') })
      const command = option(args, '--command')
      const input =
        command === undefined
          ? createInterface({ input: process.stdin, output: process.stdout, terminal: process.stdin.isTTY })
          : undefined
      try {
        for await (const line of input ?? [command as string]) {
          const answer = await shell.run(line)
          process.stdout.write(answer.stdout)
          process.stderr.write(answer.stderr)
          process.exitCode = answer.code ?? 1
        }
      } finally {
        input?.close()
        await shell.close()
      }
      break
    }
    case 'exists':
      result = await box.files.exists(first ?? '.')
      break
    case 'find':
      result = await box.files.find(first ?? '**/*')
      break
    case 'grep':
      result = await box.files.grep(first ?? '', { paths: (option(args, '--paths') ?? '.').split(',') })
      break
    case 'replace':
      result = await box.files.replace(first ?? '', args[1] ?? '', {
        paths: (option(args, '--paths') ?? '.').split(','),
      })
      break
    case 'chmod':
      result = await box.files.chmod(args[1] ?? '', Number.parseInt(first ?? '', 8))
      break
    case 'sync':
      if (!first || !args[1]) throw new Error('sync needs local and remote directories')
      result = await box.sync(first, args[1])
      break
    case 'keep-alive':
    case 'ttl':
    case 'idle': {
      if (!first) throw new Error(`${verb} needs a duration`)
      result = await (verb === 'keep-alive'
        ? box.keepAlive(duration(first))
        : verb === 'ttl'
          ? box.setTtl(duration(first))
          : box.setIdle(duration(first)))
      break
    }
    case 'metrics':
      result = await box.metrics({ since: option(args, '--since') })
      break
    case 'wait':
      if (!first) throw new Error('wait needs a state')
      result = await box.waitUntil(
        first as Parameters<Sandbox['waitUntil']>[0],
        option(args, '--timeout')
          ? duration(option(args, '--timeout') as string)
          : SDK_DEFAULTS.runTimeoutSeconds,
      )
      break
    case 'git': {
      const command = first
      const value = args[1]
      const tokenEnv = option(args, '--token-env')
      const token = tokenEnv ? process.env[tokenEnv] : undefined
      if (tokenEnv && !token) throw new Error('Token environment variable is empty')
      switch (command) {
        case 'clone':
          if (!value) throw new Error('git clone needs a URL')
          result = await box.git.clone(value, {
            path: option(args, '--path'),
            branch: option(args, '--branch'),
            depth: option(args, '--depth') ? Number(option(args, '--depth')) : undefined,
            token,
          })
          break
        case 'status':
          result = await box.git.status()
          break
        case 'diff':
          result = await box.git.diff({ staged: args.includes('--staged') })
          break
        case 'branch':
          result = await box.git.branch(value)
          break
        case 'checkout':
          result = await box.git.checkout(value ?? '', { create: args.includes('--create') })
          break
        case 'add':
          result = await box.git.add(args.slice(1))
          break
        case 'commit': {
          const author = option(args, '--author')
          const parsed = author ? /^(.*?) <([^>]+)>$/.exec(author) : undefined
          if (author && !parsed) throw new Error('author is Name <email>')
          result = await box.git.commit(value ?? '', {
            author: parsed ? { name: parsed[1] as string, email: parsed[2] as string } : undefined,
          })
          break
        }
        case 'push':
          result = await box.git.push({ token })
          break
        case 'pull':
          result = await box.git.pull()
          break
        case 'log':
          result = await box.git.log({
            limit: option(args, '--limit') ? Number(option(args, '--limit')) : undefined,
          })
          break
        default:
          throw new Error('git needs clone|status|diff|branch|checkout|add|commit|push|pull|log')
      }
      break
    }
    default:
      return { handled: false }
  }
  return { handled: true, result }
}
