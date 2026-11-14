// SPDX-License-Identifier: FSL-1.1-ALv2
/** Execute examples returned by the running welcome endpoint. SDKs use this checkout's
 * release build; the Action's real composite shell is run with its verified local CLI. */
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { StatusCodes } from 'http-status-codes'

const url = process.env.PRYX_API_URL as string
const key = process.env.PRYX_API_KEY as string
if (!url || !key) throw new Error('Hosting acceptance requires URL and key')
const publicPage = await fetch(url)
if (!publicPage.ok || (await publicPage.text()).includes(key))
  throw new Error('Welcome leaked key or missing')
const denied = await fetch(`${url}/_pryx/welcome`)
if (denied.status !== StatusCodes.UNAUTHORIZED) throw new Error('Welcome details are public')
const unlocked = await fetch(`${url}/_pryx/welcome`, { headers: { authorization: `Bearer ${key}` } })
if (!unlocked.ok) throw new Error('Platform key rejected')
const encoded = await unlocked.text()
if (encoded.includes(key)) throw new Error('Unlocked examples leaked key')
const examples = JSON.parse(encoded).examples as Array<{ id: string; code: string }>
if (examples.length !== 6) throw new Error('Caller set changed; add acceptance')
const temporary = mkdtempSync(join(tmpdir(), 'pryx-hosting-callers-'))
const env: NodeJS.ProcessEnv = { ...process.env, PATH: `${resolve(temporary)}:${process.env.PATH}` }
async function run(command: string[], code?: string, environment = env) {
  const child = Bun.spawn(command, {
    env: environment,
    stdin: code === undefined ? 'ignore' : 'pipe',
    stdout: 'pipe',
    stderr: 'pipe',
  })
  if (code !== undefined && child.stdin && typeof child.stdin !== 'number') {
    child.stdin.write(code)
    child.stdin.end()
  }
  const [output] = await Promise.all([new Response(child.stdout).text(), new Response(child.stderr).text()])
  if (await child.exited) throw new Error(`Welcome caller ${command[0]} failed`)
  return output
}
try {
  const cli = process.env.PRYX_HOSTING_CLI
  if (!cli) throw new Error('Set PRYX_HOSTING_CLI to the image CLI')
  const link = Bun.spawnSync(['ln', '-s', cli, join(temporary, 'pryx')])
  if (link.exitCode) throw new Error('Cannot expose image CLI to examples')
  for (const example of examples) {
    let output: string
    switch (example.id) {
      case 'typescript': {
        // Run from the checkout so the workspace package resolves exactly as for the SDK example.
        output = await run([process.execPath, '--eval', example.code])
        break
      }
      case 'python':
        output = await run(
          [process.env.PRYX_HOSTING_PYTHON ?? '.venv-agent-python/bin/python', '-'],
          example.code,
        )
        break
      case 'cli':
        output = await run(['sh', '-c', example.code])
        break
      case 'http': {
        output = await run(['sh', '-c', example.code])
        if (!JSON.parse(output).defaults) throw new Error('HTTP project response differs')
        output = 'hello'
        break
      }
      case 'mcp':
        output = await run([process.execPath, 'cli/src/hosting-mcp-check.ts'], example.code)
        break
      case 'action': {
        const step = (
          Bun.YAML.parse(example.code) as Array<{ uses: string; with: Record<string, string> }>
        )[0]
        if (
          !step?.uses.startsWith('cubepals/pryx/action@v') ||
          step.with.key !== `$` + '{{ secrets.PRYX_API_KEY }}'
        )
          throw new Error('Action block shape differs')
        const action = Bun.YAML.parse(await Bun.file('action/action.yml').text()) as {
          runs: { steps: Array<{ run: string }> }
        }
        const sums = await run(['sha256sum', cli])
        // The Action checks the local CLI against SHA256SUMS just as its existing CI fixture does.
        writeFileSync(join(temporary, 'SHA256SUMS'), `${sums.split(' ')[0]}  pryx\n`)
        const copied = Bun.spawnSync(['cp', cli, join(temporary, 'action-pryx')])
        if (copied.exitCode) throw new Error('Cannot stage Action CLI')
        output = await run(['bash', '-c', action.runs.steps[0]?.run ?? 'exit 1'], undefined, {
          ...env,
          PRYX_API_URL: step.with.url,
          PRYX_API_KEY: key,
          PRYX_IMAGE: step.with.image,
          PRYX_COMMAND: step.with.run,
          PRYX_SNAPSHOT: 'false',
          PRYX_FROM_SNAPSHOT: '',
          PRYX_ACTION_REF: step.uses.split('@')[1],
          PRYX_ACTION_CLI: join(temporary, 'action-pryx'),
          RUNNER_TEMP: temporary,
          GITHUB_OUTPUT: join(temporary, 'outputs'),
        })
        break
      }
      default:
        throw new Error(`Unknown welcome caller ${example.id}`)
    }
    if (!output.split('\n').some((line) => line.trim() === 'hello'))
      throw new Error(`Welcome ${example.id} output differs`)
    console.log(`Welcome ${example.id}: pass`)
  }
} finally {
  rmSync(temporary, { recursive: true, force: true })
}
