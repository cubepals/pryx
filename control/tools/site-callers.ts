// SPDX-License-Identifier: FSL-1.1-ALv2
/** Execute every extracted connect block against a supplied install. Node runs the public
 * SDK build; reference MCP clients exercise configuration without an editor/model account. */
import { createHash } from 'node:crypto'
import {
  chmodSync,
  copyFileSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  statSync,
  symlinkSync,
  writeFileSync,
} from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { Pryx } from '@cubepals/pryx'
import { z } from 'zod'
import { MINUTE_MS } from '../src/durations.ts'
import { PRYX_VERSION } from '../src/generated/release.ts'
import { WELCOME_IMAGE } from '../src/welcome-examples.ts'

type Block = { file: string; id: string; codes: Array<{ language: string; code: string }> }
const mcpEnvironmentNames = ['PRYX_API_URL', 'PRYX_API_KEY'] as const
const mcpProgram = { command: z.literal('pryx'), args: z.tuple([z.literal('mcp')]) }

export function siteMcpConfiguration(
  id: string,
  language: string,
  code: string,
  environment: NodeJS.ProcessEnv,
) {
  if (
    ![
      'connect/mcp',
      'connect/mcp/claude',
      'connect/mcp/cursor',
      'connect/mcp/codex',
      'connect/mcp/copilot',
    ].includes(id)
  )
    throw new Error('Unsupported MCP client configuration')
  const codex = id === 'connect/mcp/codex'
  if (language !== (codex ? 'toml' : 'json')) throw new Error('MCP configuration language drift')
  if (codex) {
    z.strictObject({
      mcp_servers: z.strictObject({
        pryx: z.strictObject({
          ...mcpProgram,
          env_vars: z
            .array(z.enum(mcpEnvironmentNames))
            .length(mcpEnvironmentNames.length)
            .refine((names) => new Set(names).size === mcpEnvironmentNames.length),
        }),
      }),
    }).parse(Bun.TOML.parse(code))
  } else {
    const reference = (name: string) =>
      id === 'connect/mcp/cursor' ? '${env:' + name + '}' : '${' + name + '}'
    const program = {
      ...mcpProgram,
      type: z.literal('stdio'),
      env: z.strictObject({
        PRYX_API_URL: z.literal(reference('PRYX_API_URL')),
        PRYX_API_KEY: z.literal(reference('PRYX_API_KEY')),
      }),
    }
    const server =
      id === 'connect/mcp/copilot'
        ? z.strictObject({ ...program, tools: z.tuple([z.literal('*')]) })
        : z.strictObject(program)
    z.strictObject({ mcpServers: z.strictObject({ pryx: server }) }).parse(JSON.parse(code))
  }
  const env = Object.fromEntries(
    mcpEnvironmentNames.map((name) => [name, z.string().min(1).parse(environment[name])]),
  )
  return { command: 'pryx', args: ['mcp'], env }
}

export function siteActionStep(code: string) {
  return z
    .tuple([
      z.strictObject({
        uses: z.literal(`cubepals/pryx/action@v${PRYX_VERSION}`),
        with: z.strictObject({
          url: z.literal('$' + '{{ vars.PRYX_API_URL }}'),
          key: z.literal('$' + '{{ secrets.PRYX_API_KEY }}'),
          image: z.literal('$' + '{{ vars.PRYX_IMAGE }}'),
          run: z.string().min(1),
        }),
      }),
    ])
    .parse(Bun.YAML.parse(code))[0]
}

export async function runSiteCallers(options: {
  env: NodeJS.ProcessEnv
  cli: string
  python: string
  receiver: string
  verifyWebhook: (registration: { id: string; secret: string }) => Promise<void>
}) {
  const key = options.env.PRYX_API_KEY
  const url = options.env.PRYX_API_URL
  if (!key || !url) throw new Error('Site callers require URL and project key')
  const temporary = mkdtempSync(join(tmpdir(), 'pryx-site-callers-'))
  const sdk = new Pryx({ url, key })
  const defaults = (await sdk.project()).defaults
  if (!defaults) throw new Error('Site install omitted its default tier')
  const webhookFile = join(temporary, 'webhook.json')
  const environment: NodeJS.ProcessEnv = {
    ...options.env,
    PATH: `${temporary}:${process.env.PATH}`,
    PRYX_IMAGE: defaults.isolation === 'container' ? WELCOME_IMAGE : '',
    PRYX_WEBHOOK_URL: options.receiver,
    PRYX_WEBHOOK_FILE: webhookFile,
  }
  const privateOutput: string[] = []
  let activeGuide = 'project defaults'
  const run = async (
    command: string[],
    code?: string,
    env: NodeJS.ProcessEnv = environment,
    actionMask = false,
  ) => {
    const child = Bun.spawn(command, {
      env,
      stdin: code === undefined ? 'ignore' : new TextEncoder().encode(code),
      stdout: 'pipe',
      stderr: 'pipe',
    })
    const timeout = setTimeout(() => child.kill(), MINUTE_MS)
    const [captured, stderr, status] = await Promise.all([
      new Response(child.stdout).text(),
      new Response(child.stderr).text(),
      child.exited,
    ]).finally(() => clearTimeout(timeout))
    privateOutput.push(captured, stderr)
    // The Action emits GitHub's mask command, defined by its workflow-command protocol.
    // Consume only that exact line privately; an unexpected key occurrence still fails.
    const stdout = actionMask
      ? captured
          .split('\n')
          .map((line) => (line === `::add-mask::${key}` ? '::add-mask::[redacted]' : line))
          .join('\n')
      : captured
    if (stdout.includes(key) || stderr.includes(key)) throw new Error('Caller exposed its project key')
    if (status) throw new Error(`Site caller failed: ${activeGuide}; ${command[0]}; status ${status}`)
    return stdout
  }
  const node = async (code: string) =>
    run([
      process.env.PRYX_SITE_NODE ?? 'node',
      '--input-type=module',
      '--eval',
      new Bun.Transpiler({ loader: 'ts', target: 'node' }).transformSync(code),
    ])
  try {
    symlinkSync(resolve(options.cli), join(temporary, 'pryx'))
    const blocks: Block[] = JSON.parse(readFileSync('site/.build/connect.json', 'utf8'))
    if (!blocks.length) throw new Error('No extracted connect blocks')
    for (const guide of blocks) {
      activeGuide = guide.file
      const block = guide.codes[0]
      if (guide.codes.length !== 1 || !block) throw new Error(`Unexecuted caller blocks: ${guide.file}`)
      let output: string
      if (guide.id === 'connect/webhooks') {
        if (block.language !== 'ts') throw new Error('Webhook language drift')
        output = await node(block.code)
        const file = webhookFile
        if (statSync(file).mode & 0o077) throw new Error('Webhook secret file is public')
        const registration = JSON.parse(readFileSync(file, 'utf8')) as { id: string; secret: string }
        if (
          !registration.id ||
          !registration.secret ||
          privateOutput.some((value) => value.includes(registration.secret))
        )
          throw new Error('Webhook registration omitted or exposed its secret')
        try {
          await options.verifyWebhook(registration)
        } finally {
          await run(['pryx', 'webhook', 'rm', registration.id])
        }
        if (!output.includes('Webhook registered')) throw new Error('Webhook block did not finish')
      } else if (guide.id === 'connect/rivet') {
        if (block.language !== 'ts') throw new Error('Provider language drift')
        output = await node(`${block.code}
const actor = { actorId: crypto.randomUUID(), client: () => undefined }
if (!sandboxProvider.destroy) throw new Error('Provider omitted cleanup')
const id = await sandboxProvider.create(actor)
try {
  const box = await sandboxProvider.connect(actor, id)
  if (!box) throw new Error('Provider sandbox disappeared')
  const result = await box.exec('echo hello', { cwd: box.cwd })
  if (result.exitCode !== 0 || result.stdout.trim() !== 'hello') throw new Error('Provider execution failed')
  console.log('hello')
} finally { await sandboxProvider.destroy(actor, id) }
`)
      } else if (guide.id === 'connect/typescript') {
        if (block.language !== 'ts') throw new Error('Backend language drift')
        output = await node(block.code)
      } else if (block.language === 'python' && guide.id === 'connect/python') {
        output = await run([options.python, '-'], block.code)
      } else if (block.language === 'sh' && ['connect/cli', 'connect/http'].includes(guide.id)) {
        output = await run(['sh', '-c', block.code])
        if (guide.id === 'connect/http' && !JSON.parse(output).defaults)
          throw new Error('HTTP block did not read the project')
      } else if (guide.id === 'connect/action' && block.language === 'yaml') {
        const step = siteActionStep(block.code)
        const action = Bun.YAML.parse(readFileSync('action/action.yml', 'utf8')) as {
          runs: { steps: Array<{ run: string }> }
        }
        writeFileSync(
          join(temporary, 'SHA256SUMS'),
          `${createHash('sha256').update(readFileSync(options.cli)).digest('hex')}  pryx\n`,
        )
        copyFileSync(options.cli, join(temporary, 'action-pryx'))
        chmodSync(join(temporary, 'action-pryx'), 0o755)
        output = await run(
          ['bash', '-c', action.runs.steps[0]?.run ?? 'exit 1'],
          undefined,
          {
            ...environment,
            PRYX_COMMAND: step.with.run,
            PRYX_SNAPSHOT: 'false',
            PRYX_FROM_SNAPSHOT: '',
            PRYX_ACTION_REF: step.uses.split('@')[1],
            PRYX_ACTION_CLI: join(temporary, 'action-pryx'),
            RUNNER_TEMP: temporary,
            GITHUB_OUTPUT: join(temporary, 'outputs'),
          },
          true,
        )
      } else if (guide.id === 'connect/mcp' || guide.id.startsWith('connect/mcp/')) {
        const config = siteMcpConfiguration(guide.id, block.language, block.code, environment)
        output = await run(
          [process.execPath, 'cli/src/hosting-mcp-check.ts'],
          JSON.stringify({ mcpServers: { pryx: config } }),
          { ...environment, PRYX_API_URL: undefined, PRYX_API_KEY: undefined },
        )
      } else throw new Error(`No live executor for ${guide.file}: ${block.language}`)
      if (
        !['connect/http', 'connect/webhooks'].includes(guide.id) &&
        !output.split('\n').some((line) => line.trim() === 'hello')
      )
        throw new Error(`Site caller output differs: ${guide.file}`)
      console.log(`Site ${guide.id}: pass (${defaults.isolation})`)
    }
  } finally {
    rmSync(temporary, { recursive: true, force: true })
  }
}
