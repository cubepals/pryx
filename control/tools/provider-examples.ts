// SPDX-License-Identifier: FSL-1.1-ALv2
/** Execute every connect-guide code fence and the complete provider example against CI's live install. */
import { readFileSync } from 'node:fs'

export async function runProviderExamples(options: { url: string; key: string }) {
  const env = { ...process.env, PRYX_API_URL: options.url, PRYX_API_KEY: options.key }
  const run = async (args: string[], input?: string) => {
    const child = Bun.spawn([process.execPath, ...args], {
      env,
      stdin: input === undefined ? 'ignore' : new TextEncoder().encode(input),
      stdout: 'inherit',
      stderr: 'inherit',
    })
    if (await child.exited) throw new Error(`Provider example failed: ${args.join(' ')}`)
  }
  await run(['examples/rivet-provider.ts'])
  for (const file of ['docs/integrations/rivet.md', 'site/src/content/docs/connect/rivet.md']) {
    const blocks = [...readFileSync(file, 'utf8').matchAll(/^```([^\n]*)\n([\s\S]*?)^```/gm)]
    if (blocks.length !== 2) throw new Error(`Expected two runnable connection examples in ${file}`)
    for (const block of blocks) {
      if (block[1] !== 'ts') throw new Error(`Unexecuted ${block[1]} block in ${file}`)
      await run(['-'], block[2])
    }
  }
}
