// SPDX-License-Identifier: FSL-1.1-ALv2
/** Copyable callers for this install. The key is always supplied through the caller's environment. */

import { PRYX_VERSION } from './generated/release.ts'
import type { Isolation } from './isolation.ts'

export const WELCOME_IMAGE = 'alpine:3.22'
export function welcomeExamples(origin: string, isolation: Isolation = 'container') {
  const url = new URL(origin).origin
  const image = isolation === 'container' ? WELCOME_IMAGE : ''
  const shellUrl = `'${url.replaceAll("'", "'\"'\"'")}'`
  return [
    {
      id: 'typescript',
      title: 'TypeScript',
      language: 'ts',
      setup: 'Install @cubepals/pryx. Set PRYX_API_KEY in your environment.',
      code: `import { Pryx } from '@cubepals/pryx'
const pryx = new Pryx({ url: ${JSON.stringify(url)}, key: process.env.PRYX_API_KEY! })
const box = await pryx.sandbox(${image ? JSON.stringify({ image }) : '{}'})
try {
  const result = await box.run('echo hello')
  console.log(result.stdout)
  if (result.code !== 0) throw new Error('Command failed')
} finally { await box.delete() }`,
    },
    {
      id: 'python',
      title: 'Python',
      language: 'python',
      setup: 'Install cubepals-pryx. Set PRYX_API_KEY in your environment.',
      code: `import os
from cubepals_pryx import Pryx
with Pryx(url=${JSON.stringify(url)}, key=os.environ['PRYX_API_KEY']) as pryx:
    box = pryx.sandbox(${image ? `image=${JSON.stringify(image)}` : ''})
    try:
        result = box.run('echo hello')
        print(result.stdout)
        assert result.code == 0
    finally:
        box.delete()`,
    },
    {
      id: 'cli',
      title: 'CLI',
      language: 'sh',
      setup: 'Install the pryx CLI. Set PRYX_API_KEY in your environment.',
      code: `export PRYX_API_URL=${shellUrl}\npryx run${image ? ` --image ${image}` : ''} -- echo hello`,
    },
    {
      id: 'action',
      title: 'GitHub Actions',
      language: 'yaml',
      setup: 'Add PRYX_API_KEY to repository Actions secrets. Put this step in a workflow job.',
      code: `- uses: cubepals/pryx/action@v${PRYX_VERSION}
  with:
    url: ${JSON.stringify(url)}
    key: \${{ secrets.PRYX_API_KEY }}
    image: ${JSON.stringify(image)}
    run: echo hello`,
    },
    {
      id: 'mcp',
      title: 'Agents over MCP',
      language: 'json',
      setup: `Install pryx. Merge this into .mcp.json; launch the agent with PRYX_API_KEY set. Ask it to create a sandbox${image ? ` with image ${image}` : ''} and run echo hello.`,
      code: JSON.stringify(
        { mcpServers: { pryx: { command: 'pryx', args: ['mcp'], env: { PRYX_API_URL: url } } } },
        null,
        2,
      ),
    },
    {
      id: 'http',
      title: 'Plain HTTP',
      language: 'sh',
      setup: 'Set PRYX_API_KEY in your environment. This reads your project and its defaults.',
      code: `curl --fail --silent --show-error ${shellUrl}/v1/project \\\n  --header "Authorization: Bearer $PRYX_API_KEY"`,
    },
  ]
}
