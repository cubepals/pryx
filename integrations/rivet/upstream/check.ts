// SPDX-License-Identifier: FSL-1.1-ALv2
/** Apply prepared changes to exact public sources, typecheck and bundle the added adapters,
 * and export those built modules for real CI acceptance. No upstream write or message occurs. */
import { appendFileSync, existsSync, mkdirSync, readdirSync, symlinkSync, writeFileSync } from 'node:fs'
import { join, resolve } from 'node:path'

const repository = resolve(import.meta.dir, '../../..')
const output = resolve(process.argv[2] ?? join(repository, 'target', 'prepared-rivet'))
const sourceRoot = process.env.PRYX_PATCH_SOURCE_ROOT
mkdirSync(output, { recursive: true })
async function run(args: string[], cwd = repository) {
  const child = Bun.spawn(args, { cwd, stdout: 'pipe', stderr: 'pipe' })
  const [stdout, stderr] = await Promise.all([
    new Response(child.stdout).text(),
    new Response(child.stderr).text(),
  ])
  if (await child.exited) throw new Error(`${args[0]} failed: ${stderr}`)
  return stdout.trim()
}
async function checkout(name: string, url: string, commit: string) {
  const path = join(output, name)
  if (!existsSync(join(path, '.git'))) {
    const local = sourceRoot && join(sourceRoot, `reference-${name}`)
    if (local && existsSync(join(local, '.git')))
      await run(['git', 'clone', '--shared', '--no-checkout', local, path])
    else {
      await run(['git', 'init', path])
      await run(['git', '-C', path, 'remote', 'add', 'origin', url])
      await run(['git', '-C', path, 'fetch', '--depth=1', 'origin', commit])
    }
    await run(['git', '-C', path, 'checkout', '--detach', commit])
  }
  if ((await run(['git', '-C', path, 'rev-parse', 'HEAD'])) !== commit)
    throw new Error('Public patch base changed')
  const patch = join(import.meta.dir, name, 'change.patch')
  await run(['git', '-C', path, 'apply', '--check', patch])
  await run(['git', '-C', path, 'apply', patch])
  const modules = join(path, 'node_modules')
  mkdirSync(modules, { recursive: true })
  const available = join(repository, 'integrations', 'rivet', 'node_modules')
  for (const name of readdirSync(available)) {
    if (name.startsWith('.')) continue
    symlinkSync(join(available, name), join(modules, name), 'dir')
  }
  return path
}
const sandboxCommit = 'f28965688227b981aae908a88b4a96770cf45e6f'
const agentosCommit = '4662e362fb93adcfbfc86bfd51b51855d463f371'
const sandbox = await checkout('sandbox-agent', 'https://github.com/rivet-dev/agents', sandboxCommit)
const agentos = await checkout('agentos', 'https://github.com/rivet-dev/agentos', agentosCommit)
const provider = join(sandbox, 'sandbox-agent/sdks/typescript/src/providers/pryx.ts')
const boundary = join(agentos, 'packages/agentos-sandbox/src/pryx.ts')
const config = join(output, 'tsconfig.json')
writeFileSync(
  config,
  JSON.stringify(
    {
      extends: join(repository, 'tsconfig.json'),
      compilerOptions: {
        url: repository,
        paths: { 'sandbox-agent/pryx': [provider] },
        typeRoots: [join(repository, 'node_modules/@types')],
      },
      include: [provider, boundary],
      exclude: [],
    },
    null,
    2,
  ),
)
await run([
  Bun.which('node') ?? process.execPath,
  join(repository, 'node_modules/typescript/bin/tsc'),
  '--project',
  config,
])
for (const [name, entry] of [
  ['provider', provider],
  ['agentos', boundary],
] as const) {
  const built = await Bun.build({
    entrypoints: [entry],
    target: 'node',
    packages: 'bundle',
    outdir: join(output, 'built', name),
    plugins: [
      {
        name: 'public-pryx-provider',
        setup(builder) {
          builder.onResolve({ filter: /^sandbox-agent\/pryx$/ }, () => ({ path: provider }))
        },
      },
    ],
  })
  if (!built.success) throw new Error(`Public ${name} adapter did not build`)
}
const artifacts = {
  provider: join(output, 'built/provider/pryx.js'),
  agentos: join(output, 'built/agentos/pryx.js'),
}
writeFileSync(
  join(output, 'result.json'),
  JSON.stringify({ sandboxCommit, agentosCommit, ...artifacts }, null, 2),
)
if (process.env.GITHUB_ENV)
  appendFileSync(
    process.env.GITHUB_ENV,
    `PRYX_PATCH_PROVIDER=${artifacts.provider}\nPRYX_PATCH_AGENTOS=${artifacts.agentos}\n`,
  )
console.log(JSON.stringify({ sandboxCommit, agentosCommit, typecheck: true, built: Object.keys(artifacts) }))
