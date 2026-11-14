// SPDX-License-Identifier: FSL-1.1-ALv2
/** Weekly compatibility probe uses the newest published adapter types under the recorded licence. */
export {}

const response = await fetch('https://registry.npmjs.org/@rivet-dev%2fsandbox-adapter/latest')
if (!response.ok) throw new Error(`Sandbox adapter registry returned ${response.status}`)
const latest = (await response.json()) as { version: string; license: string }
if (latest.license !== 'Apache-2.0') throw new Error('Review the new sandbox adapter licence before use')
if (!/^\d+\.\d+\.\d+(?:-[\w.-]+)?$/.test(latest.version)) throw new Error('Invalid adapter version')
const child = Bun.spawn(
  [
    process.execPath,
    'add',
    '--dev',
    '--exact',
    '--cwd',
    'sdk/typescript',
    `@rivet-dev/sandbox-adapter@${latest.version}`,
  ],
  { stdout: 'inherit', stderr: 'inherit' },
)
if (await child.exited) throw new Error('Newest sandbox adapter types could not be installed')
const checked = Bun.spawn([process.execPath, 'run', 'typecheck'], { stdout: 'inherit', stderr: 'inherit' })
if (await checked.exited) throw new Error(`Provider no longer satisfies sandbox-adapter ${latest.version}`)
console.log(`Provider satisfies sandbox-adapter ${latest.version}`)
