// SPDX-License-Identifier: FSL-1.1-ALv2
/** Run a README's actual code fences, so documentation examples are the code exercised in CI. */
import { readFileSync } from 'node:fs'

const [file, language, ...command] = process.argv.slice(2)
if (!file || !language || !command.length) throw new Error('readme-example.ts FILE LANGUAGE COMMAND...')
const blocks = [...readFileSync(file, 'utf8').matchAll(/^```([^\n]*)\n([\s\S]*?)^```/gm)].filter(
  (match) => match[1] === language,
)
if (!blocks.length) throw new Error('No runnable README examples')
for (const block of blocks) {
  const process = Bun.spawn(command, {
    stdin: new TextEncoder().encode(block[2] as string),
    stdout: 'inherit',
    stderr: 'inherit',
    cwd: file.startsWith('sdk/typescript/') ? 'sdk/typescript' : undefined,
  })
  if ((await process.exited) !== 0) throw new Error(`${file} example failed`)
}
