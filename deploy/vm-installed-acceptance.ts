// SPDX-License-Identifier: FSL-1.1-ALv2
/** Exercise an actual signed VM installation through both SDKs and the edge: the ordinary
 * user's, or, with PRYX_VM_INSTALLATION=server, root's server preset and its VM worker.
 * Private installation records supply credentials; output contains only public proof results.
 * No KVM, network or isolation capability is inferred from a successful artifact build. */
import assert from 'node:assert/strict'
import { mkdtempSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs'
import { homedir, networkInterfaces, release } from 'node:os'
import { dirname, join } from 'node:path'
import { readClientConfig } from '../cli/src/client-config.ts'
import { readInstallation } from '../cli/src/up-local-record.ts'
import { userPaths } from '../cli/src/up-paths.ts'
import { AppRequests } from '../control/e2e/app-requests.ts'
import { until } from '../control/e2e/waits.ts'
import { MINUTE_MS, SECOND_MS } from '../control/src/durations.ts'
import nodeSchema from '../openapi/node-writes.openapi.json'
import { Pryx, type Sandbox } from '../sdk/typescript/src/index.ts'
import inputs from '../templates/vm-tools/image-inputs.json'
import { hostProbe } from './host-probe.ts'
import { vmMeasurements } from './vm-installed-measurements.ts'

/** The server preset's node as deploy/pryxd.service starts it; nothing exports these paths. */
const SERVER_NODE = { binary: '/usr/local/bin/pryxd', config: '/etc/pryxd/pryxd.toml' }

/** The installation under test. The server preset writes its CLI credential to the home of the
 * user who ran sudo, so a root run is handed it in PRYX_API_URL and PRYX_API_KEY. Only the
 * ordinary user's installation is measured: docs/tiers.md's figures are that installation's. */
function installation() {
  if (process.env.PRYX_VM_INSTALLATION === 'server') {
    const { PRYX_API_URL: url, PRYX_API_KEY: key } = process.env
    assert(url && key && process.getuid?.() === 0)
    const node = Bun.TOML.parse(readFileSync(SERVER_NODE.config, 'utf8')) as {
      runtime?: unknown
      workloads?: { allowed_images?: unknown }
    }
    assert.equal(node.runtime, 'auto')
    // The install itself admitted the VM bundle's images, by digest (cli/src/up-server.ts).
    const allowed = node.workloads?.allowed_images
    assert(Array.isArray(allowed) && inputs.every((image) => allowed.includes(image.reference)))
    return {
      credential: { url, key },
      apiUrl: url,
      descriptor: join(dirname(nodeSchema['x-pryx-agent'].defaultBinary), 'vm-bundle.json'),
      configured: SERVER_NODE.config,
      nodeBinary: SERVER_NODE.binary,
      measure: async (_: Pryx) => {},
    }
  }
  const credential = readClientConfig()
  const installed = readInstallation(userPaths())
  assert(credential && installed && process.getuid?.() !== 0)
  assert.equal(installed.runtime, 'auto')
  return {
    credential,
    apiUrl: installed.apiUrl,
    descriptor: join(installed.bundle, 'bin/vm-bundle.json'),
    configured: join(installed.paths.config, 'pryxd.toml'),
    nodeBinary: join(installed.bundle, 'bin/pryxd'),
    measure: (sdk: Pryx) => vmMeasurements(sdk, installed),
  }
}

const { credential, apiUrl, descriptor, configured, nodeBinary, measure } = installation()
assert.equal(statSync(descriptor).mode & 0o777, 0o444)
const probe = await hostProbe(nodeBinary, process.env, configured)
assert.equal(probe.tiers?.vm?.state, 'on')
const sdk = new Pryx(credential)
const apps = new AppRequests(apiUrl)
const privateRoot = mkdtempSync(join(homedir(), 'vm-host-only-'))
const hostOnly = join(privateRoot, 'private')
writeFileSync(hostOnly, crypto.randomUUID(), { mode: 0o600 })

function nodePid() {
  const nodes = readdirSync('/proc').flatMap((name) => {
    if (!/^\d+$/.test(name)) return []
    try {
      const status = readFileSync(`/proc/${name}/status`, 'utf8')
      if (Number(status.match(/^Uid:\s+(\d+)/m)?.[1]) !== process.getuid?.()) return []
      // Tini carries the wrapper path too; only the loader carries the ELF payload.
      const argv = readFileSync(`/proc/${name}/cmdline`, 'utf8').split('\0')
      return (argv[0] === nodeBinary || argv.includes(`${nodeBinary}.real`)) &&
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
    const response = await apps.get(`http://${endpoint.hostname}/`, AbortSignal.timeout(30 * SECOND_MS))
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

/** The default network's firewall, whichever user runs the node: the guest opens a TCP
 * connection to a public address, and none to a listener on the host's own non-loopback address
 * that the host itself reaches. The image's bash opens the connections. */
async function hostListenerDenied(box: Sandbox) {
  const address = Object.values(networkInterfaces())
    .flat()
    .find((item) => item?.family === 'IPv4' && !item.internal)?.address
  assert(address, 'The host has no non-loopback IPv4 address to test')
  const listener = Bun.listen({
    hostname: address,
    port: 0,
    socket: { data() {}, open: (socket) => void socket.end() },
  })
  try {
    const own = await Bun.connect({ hostname: address, port: listener.port, socket: { data() {} } })
    own.end()
    const connect = (host: string, port: number) =>
      box.run(['timeout', '10', 'bash', '-c', 'exec 3<>"/dev/tcp/$1/$2"', 'vm-firewall', host, String(port)])
    const outside = await connect('1.1.1.1', 443)
    assert.equal(outside.code, 0, `The guest reached no public address: ${outside.stderr}`)
    assert.notEqual((await connect(address, listener.port)).code, 0, 'The guest reached a host listener')
  } finally {
    listener.stop(true)
  }
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
    entrypoint: [
      '/bin/sh',
      '-c',
      'while [ ! -x /data/vm-http ]; do sleep 1; done; cd /data && exec /data/vm-http',
    ],
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
    if (network === 'internet') await hostListenerDenied(box)
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
    // Wake can serve one held request before every edge acknowledges the new route list.
    await until('installed VM running view after edge cold wake', async () => {
      await box.refresh()
      return box.state === 'running'
    })
    assert.equal(box.state, 'running')
    assert.equal(await box.read('index.html'), proof)
    const snapshot = await box.snapshot()
    try {
      const fork = await snapshot.fork({
        network,
        sparePorts: 0,
        ports: [{ name: 'web', port: 3000, audience: 'public', front: 'http' }],
      })
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
        root: process.getuid?.() === 0,
        image: image.image,
        isolation: 'vm',
        network,
        hostListenerDenied: network === 'internet',
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

// The node's default process tier has no hostname gateways; control must still send the VM
// tier's own policy, and the guest's gateways must admit one host and refuse the rest.
async function egress(image: (typeof inputs)[number]) {
  const box = await sdk.sandbox({
    isolation: 'vm',
    image: image.reference,
    units: 1,
    idle: (10 * MINUTE_MS) / SECOND_MS,
    sparePorts: 0,
    egress: { allow: ['example.com'], deny: ['example.org'] },
    require: ['own-kernel', 'private-network'],
  })
  try {
    assert.equal(box.isolation, 'vm')
    const get = (url: string, seconds: number) =>
      box.run(['wget', '-q', '-T', String(seconds), '-O', '/dev/null', url])
    const allowed = await get('https://example.com/', 20)
    assert.equal(allowed.code, 0, `Allowed VM egress failed: ${allowed.stderr}`)
    for (const url of ['https://example.org/', 'http://example.org/', 'http://1.1.1.1/'])
      assert.notEqual((await get(url, 5)).code, 0, `VM egress admitted ${url}`)
    console.log(
      JSON.stringify({
        installed: true,
        root: process.getuid?.() === 0,
        image: image.image,
        isolation: 'vm',
        hostnameEgress: true,
      }),
    )
  } finally {
    assert.equal((await box.delete()).existed, true)
  }
}

try {
  for (const [index, image] of inputs.entries()) await lifecycle(image, index === 0 ? 'none' : 'internet')
  assert(inputs[0])
  await egress(inputs[0])
  await measure(sdk)
} finally {
  apps.close()
  rmSync(privateRoot, { recursive: true })
}
