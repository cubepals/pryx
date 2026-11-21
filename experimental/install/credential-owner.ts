// SPDX-License-Identifier: FSL-1.1-ALv2
/** Two-account fixture for the signed handoff. Only the provisioning workload is substituted. */
import assert from 'node:assert/strict'
import { readdirSync } from 'node:fs'
import { join } from 'node:path'
import { callerHomeAndDropPrivileges, writeClientConfig } from '../../cli/src/client-config.ts'
import { handoff } from '../../cli/src/experimental-release.ts'

const [mode] = process.argv.slice(2)
assert.equal(process.getuid?.(), 0)
process.umask(0o002)
assert.ok(mode && ['ready', 'failed', 'killed', 'foreground'].includes(mode))
const choice = mode === 'failed' ? [] : ['--feature', 'self-check']
await handoff(true, 'up', ['--experimental', ...choice], async (_: string[], ready: () => void) => {
  assert.ok(!readdirSync('/tmp').some((name) => name.startsWith('pryx-experimental-')))
  if (mode === 'killed') process.kill(process.pid, 'SIGKILL')
  const home = callerHomeAndDropPrivileges()
  assert.equal(process.getuid?.(), Number(process.env.SUDO_UID))
  writeClientConfig(
    { url: 'http://127.0.0.1:1', key: 'disposable-fixture' },
    join(home, '.config/pryx/client.json'),
  )
  if (mode === 'failed') throw new Error('stock setup failed after its credential drop')
  if (mode === 'foreground') {
    ready()
    assert.ok(readdirSync(join(home, '.pryx-experimental')).includes('active'))
  }
})
