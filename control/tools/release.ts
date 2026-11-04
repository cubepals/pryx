// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Build the CLI/control/edge binaries and deterministic runtime assets for the same Cargo release. */
import { cpSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'

const root = new URL('../../', import.meta.url).pathname
const version = /^version = "([^"]+)"/m.exec(readFileSync(join(root, 'Cargo.toml'), 'utf8'))?.[1]
if (!version) throw new Error('Cargo version missing')
const sdkVersion = JSON.parse(readFileSync(join(root, 'sdk/typescript/package.json'), 'utf8')).version
const pythonVersion = /^version = "([^"]+)"/m.exec(
  readFileSync(join(root, 'sdk/python/pyproject.toml'), 'utf8'),
)?.[1]
if (sdkVersion !== version || pythonVersion !== version) throw new Error('SDK versions must match Cargo.toml')
const target = join(root, 'control/src/generated/release.ts')
const contents = `// SPDX-License-Identifier: FSL-1.1-ALv2\n// Generated from Cargo.toml by control/tools/release.ts.\nexport const PRYX_VERSION = '${version}'\n`
const mode = process.argv[2] ?? 'check'
if (mode === 'check') {
  if (readFileSync(target, 'utf8') !== contents) throw new Error('Release version drift')
  process.exit(0)
}
mkdirSync(join(root, 'control/src/generated'), { recursive: true })
writeFileSync(target, contents)
if (mode === 'generate') process.exit(0)
if (mode !== 'build') throw new Error('release.ts check|generate|build')
mkdirSync(join(root, 'dist'), { recursive: true })
for (const [source, name] of [
  ['cli/src/main.ts', 'pryx'],
  ['control/src/main.ts', 'pryx-control'],
  ['edge/src/agent.ts', 'pryx-edge'],
]) {
  const result = Bun.spawnSync(['bun', 'build', '--compile', source as string, '--outfile', `dist/${name}`], {
    cwd: root,
    stdout: 'inherit',
    stderr: 'inherit',
  })
  if (result.exitCode !== 0) throw new Error(`${name} compilation failed`)
}
const sdk = Bun.spawnSync(
  [
    'bun',
    'build',
    'sdk/typescript/src/index.ts',
    '--target',
    'node',
    '--format',
    'esm',
    '--outfile',
    'sdk/typescript/dist/index.js',
  ],
  { cwd: root, stdout: 'inherit', stderr: 'inherit' },
)
if (sdk.exitCode !== 0) throw new Error('SDK JavaScript build failed')
const assets = join(root, 'target/release-assets/control')
mkdirSync(assets, { recursive: true })
cpSync(join(root, 'control/src/schema/migrations'), join(assets, 'migrations'), { recursive: true })
for (const file of ['join.sh', 'daemon.json']) cpSync(join(root, 'control/src', file), join(assets, file))
const result = Bun.spawnSync(
  [
    'tar',
    '--sort=name',
    '--mtime=@0',
    '--owner=0',
    '--group=0',
    '--numeric-owner',
    '-czf',
    join(root, 'dist/pryx-control-assets.tar.gz'),
    '-C',
    join(root, 'target/release-assets'),
    'control',
  ],
  { stdout: 'inherit', stderr: 'inherit' },
)
if (result.exitCode !== 0) throw new Error('Runtime asset archive failed')
