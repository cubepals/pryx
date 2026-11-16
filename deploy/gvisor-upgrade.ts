// SPDX-License-Identifier: FSL-1.1-ALv2
/** Real Docker replay of a runsc pin upgrade with a sandbox alive. Actual host executables
 * establish which version serves each sandbox; retained executable/sidecar directories keep
 * old sandboxes usable while the server preset selects its new pin for new launches. Source
 * and payload verification remain in the installer; this fixture publishes no release. */
import assert from 'node:assert/strict'
import { existsSync, readFileSync, readlinkSync } from 'node:fs'
import { dirname, join, sep } from 'node:path'
import previous from '../.github/gvisor-upgrade.json'
import {
  configureHostRuntime,
  GVISOR_PREFIX,
  gvisorRuntimes,
  installGvisor,
  waitForGvisorRuntime,
} from '../cli/src/up-host.ts'
import gvisor from './gvisor.json'

async function output(args: string[]) {
  const child = Bun.spawn(args, { stdout: 'pipe', stderr: 'pipe' })
  const [code, stdout, stderr] = await Promise.all([
    child.exited,
    new Response(child.stdout).text(),
    new Response(child.stderr).text(),
  ])
  assert.equal(code, 0, `${args[0]} failed: ${stderr}`)
  return stdout.trim()
}
const run = async (args: string[]) => {
  await output(args)
}
const paths = async () => JSON.parse(await output(['docker', 'info', '--format', '{{json .Runtimes}}']))
const prior = join(GVISOR_PREFIX, previous.version)
await installGvisor(run, prior, previous.url, previous)
const config = '/etc/docker/daemon.json'
const existing = JSON.parse(readFileSync(config, 'utf8'))
await Bun.write(config, JSON.stringify({ ...existing, runtimes: gvisorRuntimes(existing, prior) }, null, 2))
await run(['systemctl', 'reload', 'docker'])
await waitForGvisorRuntime(join(prior, 'runsc'))
const before = (await paths()).runsc.path as string
assert.equal(before, join(prior, 'runsc'))
const oldName = `pryx-gvisor-old-${process.pid}`
const newName = `pryx-gvisor-new-${process.pid}`
const created: string[] = []
const startTime = (stat: string) =>
  stat
    .slice(stat.lastIndexOf(')') + 1)
    .trim()
    .split(/\s+/)[19]
async function launch(name: string) {
  await output([
    'docker',
    'run',
    '-d',
    '--name',
    name,
    '--runtime',
    'runsc',
    '--memory',
    '256m',
    '--user',
    '1000:1000',
    '--cap-drop',
    'ALL',
    '--security-opt',
    'no-new-privileges:true',
    'alpine:3.22',
    'sh',
    '-c',
    'echo kept > /tmp/pryx-upgrade; exec sleep 300',
  ])
  created.push(name)
}
async function version(name: string, runtime: string) {
  const pid = Number(await output(['docker', 'inspect', '--format', '{{.State.Pid}}', name]))
  assert(Number.isSafeInteger(pid) && pid > 1)
  const executable = readlinkSync(`/proc/${pid}/exe`)
  assert(
    executable === runtime || executable.startsWith(join(dirname(runtime), 'gvisor-bin') + sep),
    `unexpected sandbox executable: ${executable}`,
  )
  return output([`/proc/${pid}/exe`, '--version'])
}
try {
  await launch(oldName)
  assert((await version(oldName, before)).includes(`runsc version ${previous.version}\n`))
  const init = startTime(await output(['docker', 'exec', oldName, 'cat', '/proc/1/stat']))
  await configureHostRuntime(run)
  const after = (await paths()).runsc.path as string
  assert.equal(after, join(GVISOR_PREFIX, gvisor.version, 'runsc'))
  assert.notEqual(gvisor.version, previous.version)
  assert.notEqual(after, before)
  assert(existsSync(before), 'the previous executable remains available')
  assert(existsSync(join(dirname(before), 'gvisor-bin')), 'the previous sidecars remain available')
  assert.equal(await output(['docker', 'exec', oldName, 'cat', '/tmp/pryx-upgrade']), 'kept')
  assert.equal(startTime(await output(['docker', 'exec', oldName, 'cat', '/proc/1/stat'])), init)
  assert((await version(oldName, before)).includes(`runsc version ${previous.version}\n`))
  await launch(newName)
  assert((await version(newName, after)).includes(`runsc version ${gvisor.version}\n`))
  assert.equal(await output(['docker', 'exec', newName, 'cat', '/tmp/pryx-upgrade']), 'kept')
  console.log(`gVisor ${previous.version} sandbox survived upgrade; new sandbox uses ${gvisor.version}`)
} finally {
  for (const name of created.reverse()) await output(['docker', 'rm', '-f', name])
}
