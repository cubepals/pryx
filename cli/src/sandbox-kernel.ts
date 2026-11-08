// SPDX-License-Identifier: FSL-1.1-ALv2
/** Python snippets and a multiline REPL share a live sandbox kernel. Jupyter decides whether
 * input is complete; terminal interruptions signal that kernel without replaying the snippet. */
import { createInterface } from 'node:readline/promises'
import type { Pryx } from '@cubepals/pryx'

export async function python(client: Pryx, args: string[]) {
  const codeAt = args.indexOf('-c')
  if (codeAt >= 0 && args[codeAt + 1] === undefined) throw new Error('-c needs Python code')
  const id = args[0] && !args[0].startsWith('-') ? args[0] : undefined
  const contextAt = args.indexOf('--context')
  if (contextAt >= 0 && !args[contextAt + 1]) throw new Error('--context needs a name')
  const box = await client.sandbox(id ?? { template: 'python-interpreter', isolation: 'container' })
  try {
    const kernel = await box.python({ context: contextAt < 0 ? undefined : args[contextAt + 1] })
    const interrupt = () => {
      void kernel.interrupt().catch((error) => console.error(error.message))
    }
    process.on('SIGINT', interrupt)
    try {
      const run = async (code: string) => {
        let failed = false
        for await (const event of kernel.stream(code)) {
          if (event.type === 'stream')
            (event.stream === 'stdout' ? process.stdout : process.stderr).write(event.text)
          else if (event.type === 'result') {
            const text = (event.data as Record<string, unknown>)['text/plain']
            if (typeof text === 'string') console.log(text)
          } else if (event.type === 'error') {
            console.error(event.traceback.join('\n'))
            failed = true
          }
        }
        if (failed && codeAt >= 0) process.exitCode = 1
      }
      if (codeAt >= 0) {
        await run(args[codeAt + 1] as string)
        return
      }
      const input = createInterface({
        input: process.stdin,
        output: process.stdout,
        terminal: process.stdin.isTTY,
      })
      input.on('SIGINT', interrupt)
      let code = ''
      if (process.stdin.isTTY) {
        input.setPrompt('>>> ')
        input.prompt()
      }
      try {
        for await (const line of input) {
          code += `${line}\n`
          const completion = await kernel.complete(code)
          if (completion.status !== 'incomplete') {
            await run(code)
            code = ''
          }
          if (process.stdin.isTTY) {
            input.setPrompt(code ? (completion.indent ?? '... ') : '>>> ')
            input.prompt()
          }
        }
        if (code.trim()) await run(code)
      } finally {
        input.close()
      }
    } finally {
      process.off('SIGINT', interrupt)
    }
  } finally {
    if (!id) await box.delete()
  }
}
