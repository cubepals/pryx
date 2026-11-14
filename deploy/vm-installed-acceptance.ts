// SPDX-License-Identifier: FSL-1.1-ALv2
/** Exercise actual signed ordinary-user VM installation through both SDKs and the edge.
 * Private installation records supply credentials; output contains only public proof results.
 * No KVM, network or isolation capability is inferred from a successful artifact build. */
import assert from 'node:assert/strict'
import { mkdtempSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs'
import { homedir, release } from 'node:os'
import { join } from 'node:path'
import { readClientConfig } from '../cli/src/client-config.ts'
import { readInstallation } from '../cli/src/up-local-record.ts'
import { userPaths } from '../cli/src/up-paths.ts'
import { AppRequests } from '../control/e2e/app-requests.ts'
import { until } from '../control/e2e/waits.ts'
import { MINUTE_MS, SECOND_MS } from '../control/src/durations.ts'
import { Pryx, type Sandbox } from '../sdk/typescript/src/index.ts'
import inputs from '../templates/vm-tools/image-inputs.json'
import { hostProbe } from './host-probe.ts'
import { vmMeasurements } from './vm-installed-measurements.ts'

const credential = readClientConfig()
const installed = readInstallation(userPaths())
assert(credential && installed && process.getuid?.() !== 0)
assert.equal(installed.runtime, 'auto')
const descriptor = join(installed.bundle, 'bin/vm-bundle.json')
assert.equal(statSync(descriptor).mode & 0o777, 0o444)
const configured = join(installed.paths.config, 'pryxd.toml')
const nodeBinary = join(installed.bundle, 'bin/pryxd')
const probe = await hostProbe(nodeBinary, process.env, configured)
assert.equal(probe.tiers?.vm?.state, 'on')
const sdk = new Pryx(credential)
const apps = new AppRequests(installed.apiUrl)
const privateRoot = mkdtempSync(join(homedir(), 'vm-host-only-'))
const hostOnly = join(privateRoot, 'private')
writeFileSync(hostOnly, crypto.randomUUID(), { mode: 0o600 })

function nodePid() {
  const nodes = readdirSync('/proc').flatMap((name) => {
    if (!/^\d+$/.test(name)) return []
    try {
      const status = readFileSync(`/proc/${name}/status`, 'utf8')
      if (Number(status.match(/^Uid:\s+(\d+)/m)?.[1]) !== process.getuid?.()) return []
      const argv = readFileSync(`/proc/${name}/cmdline`, 'utf8').split('\0')
      return (argv.includes(nodeBinary) || argv.includes(`${nodeBinary}.real`)) &&
        argv.includes('serve') &&
        argv.includes(configured)
        ? [Number(name)]
        : []
    } catch (error) {
      if (['ENOENT', 'ESRCH'].includes((error as NodeJS.ErrnoException).code ?? '')) return []
      throw error
    }
  })
  assert.equal(nodes.length, 1, 'Expected the actual installed node process')
  return nodes[0] as number
}

async function edge(box: Sandbox, proof: string) {
  const endpoint = (await box.endpoints()).find((item) => item.name === 'web')
  assert(endpoint?.hostname)
  await until('retained VM application through installed edge', async () => {
    const response = await apps.get(
      `http://${endpoint.hostname}/index.html`,
      AbortSignal.timeout(30 * SECOND_MS),
    )
    return response.ok && (await response.text()) === proof
  })
}

async function python(box: Sandbox) {
  assert(credential)
  const child = Bun.spawn([process.env.PRYX_PYTHON ?? 'python3', 'sdk/python/e2e/vm.py'], {
    env: {
      ...process.env,
      PRYX_API_URL: credential.url,
      PRYX_API_KEY: credential.key,
      PRYX_VM_SANDBOX: box.id,
    },
    stdout: 'pipe',
    stderr: 'pipe',
  })
  const [code, output, error] = await Promise.all([
    child.exited,
    new Response(child.stdout).text(),
    new Response(child.stderr).text(),
  ])
  assert.equal(code, 0, `Python VM acceptance failed: ${error.slice(-2000)}`)
  assert.equal(output, '')
}

async function lifecycle(image: (typeof inputs)[number], network: 'internet' | 'none') {
  const box = await sdk.sandbox({
    isolation: 'vm',
    image: image.reference,
    units: 1,
    idle: (10 * MINUTE_MS) / SECOND_MS,
    network,
    sparePorts: 0,
    ports: [{ name: 'web', port: 3000, audience: 'public', front: 'http' }],
    entrypoint: ['/bin/sh', '-c', 'while [ ! -x /data/vm-http ]; do sleep 1; done; exec /data/vm-http'],
    require: ['own-kernel', 'memory-limit', 'cpu-limit', 'private-network', 'private-pids'],
  })
  try {
    assert.equal(box.isolation, 'vm')
    assert.equal(box.state, 'running')
    assert.equal(box.guarantees?.['memory-sleep'], false)
    const daemon = nodePid()
    const boundary = await box.run([
      '/bin/sh',
      '-c',
      'test ! -r "$1" && ! kill -0 "$2" 2>/dev/null && uname -r',
      'vm-boundary',
      hostOnly,
      String(daemon),
    ])
    assert.equal(boundary.code, 0)
    assert.notEqual(boundary.stdout.trim(), release())
    process.kill(daemon, 0)
    const proof = crypto.randomUUID()
    await box.write('index.html', proof)
    assert.equal(await box.read('index.html'), proof)
    await box.write('vm-http', readFileSync('vm-http'))
    await box.files.chmod('vm-http', 0o700)
    await edge(box, proof)
    await python(box)
    await box.sleep()
    assert(['sleeping', 'resting'].includes(box.state))
    // A safe application GET must trigger cold wake; the entrypoint restarts from retained disk.
    await edge(box, proof)
    await box.refresh()
    assert.equal(box.state, 'running')
    assert.equal(await box.read('index.html'), proof)
    const snapshot = await box.snapshot()
    try {
      const fork = await snapshot.fork()
      try {
        assert.equal(fork.isolation, 'vm')
        assert.equal(await fork.read('index.html'), proof)
        assert.equal((await fork.run(['/bin/sh', '-c', 'true'])).code, 0)
        await edge(fork, proof)
      } finally {
        assert.equal((await fork.delete()).existed, true)
      }
    } finally {
      await snapshot.delete()
    }
    console.log(
      JSON.stringify({
        installed: true,
        image: image.image,
        isolation: 'vm',
        network,
        bothPythonModes: true,
        fileRetention: true,
        edgePort: true,
        edgeColdWake: true,
        snapshotFork: true,
        hostFilesDenied: true,
        nodeSignalDenied: true,
        separateKernel: true,
      }),
    )
  } finally {
    assert.equal((await box.delete()).existed, true)
  }
}

try {
  for (const [index, image] of inputs.entries()) await lifecycle(image, index === 0 ? 'none' : 'internet')
  await vmMeasurements(sdk, installed)
} finally {
  apps.close()
  rmSync(privateRoot, { recursive: true })
}
