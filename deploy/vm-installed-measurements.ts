// SPDX-License-Identifier: FSL-1.1-ALv2
/** Thirty real installed VM cohorts. Timed operations use the public API and authenticated agent;
 * idle memory uses the unchanged aggregate process-tree PSS sampler through a CI-only Unix fixture.
 * Raw/partial samples stay in CI artifacts; this records neither credentials nor runtime journals. */
import assert from 'node:assert/strict'
import { writeFileSync } from 'node:fs'
import { createConnection } from 'node:net'
import { cpus, homedir, release, totalmem } from 'node:os'
import { join } from 'node:path'
import { type LocalInstallation, supervisor } from '../cli/src/up-local-record.ts'
import { writeBaseline } from '../control/bench/baseline-report.ts'
import { baselineFailureCode, collectBaseline } from '../control/bench/pryx-baseline.ts'
import { snapshots } from '../control/bench/tiers/operations.ts'
import { DATA_BYTES, measure, percentile, REPETITIONS, type Samples } from '../control/bench/tiers/report.ts'
import { MINUTE_MS, SECOND_MS } from '../control/src/durations.ts'
import { Pryx, type Sandbox } from '../sdk/typescript/src/index.ts'
import inputs from '../templates/vm-tools/image-inputs.json'

const VM_HELPER_ROLES = ['launcher', 'firecracker', 'network-proxy', 'vm-transport'] as const
type HelperPss = Record<(typeof VM_HELPER_ROLES)[number], { pssBytes: number | null; processes: number }>

async function memoryReport(socket: string, root: number, vmHelpers = false) {
  const observation = await new Promise<string>((resolve, reject) => {
    const client = createConnection(socket)
    let bytes = ''
    client.setTimeout(30 * SECOND_MS, () => client.destroy(new Error('VM PSS observation deadline')))
    client.on('error', reject)
    client.on('connect', () => client.end(JSON.stringify({ root, ...(vmHelpers ? { vmHelpers } : {}) })))
    client.on('data', (part) => {
      bytes += part.toString()
      if (bytes.length > 64 * 1024) client.destroy(new Error('VM PSS observation exceeds its bound'))
    })
    client.on('end', () => resolve(bytes))
  })
  const value = JSON.parse(observation) as { pssBytes: number; processes: number; vmHelpers?: HelperPss }
  assert(typeof value.pssBytes === 'number' && value.pssBytes > 0 && Number.isFinite(value.pssBytes))
  assert(typeof value.processes === 'number' && value.processes >= 1)
  return value
}

async function memory(socket: string, root: number) {
  return (await memoryReport(socket, root)).pssBytes
}

function vmRequest() {
  assert(inputs[0])
  return {
    isolation: 'vm' as const,
    image: inputs[0].reference,
    units: 1,
    idle: (10 * MINUTE_MS) / SECOND_MS,
    sparePorts: 0,
  }
}

function make(sdk: Pryx) {
  return sdk.sandbox(vmRequest())
}

async function helperMemory(sdk: Pryx, socket: string, root: number, samples: Samples) {
  const repetitions = 5
  for (const role of VM_HELPER_ROLES) {
    samples[`vm_${role}_processes`] = []
    samples[`vm_${role}_pss_bytes_per_process`] = []
  }
  for (let sample = 0; sample < repetitions; sample++) {
    const before = (await memoryReport(socket, root, true)).vmHelpers
    assert(
      before && VM_HELPER_ROLES.every((role) => before[role].processes === 0),
      'VM context contains a live helper',
    )
    const box = await make(sdk)
    try {
      assert.equal(box.state, 'running')
      const after = (await memoryReport(socket, root, true)).vmHelpers
      assert(after, 'VM helper PSS observation missing')
      for (const role of VM_HELPER_ROLES) {
        const { pssBytes, processes }: HelperPss[typeof role] = after[role]
        assert(Number.isInteger(processes) && processes >= 0)
        samples[`vm_${role}_processes`]?.push(processes)
        if (processes === 0) assert.equal(pssBytes, null)
        else {
          assert(typeof pssBytes === 'number' && Number.isFinite(pssBytes) && pssBytes >= 0)
          samples[`vm_${role}_pss_bytes_per_process`]?.push(pssBytes / processes)
        }
      }
    } finally {
      await box.delete()
    }
  }
}

/** Each cohort as docs/tiers.md names it, so the job log carries every figure the artifact does. */
const FIGURES = [
  ['cold_create_running_ms', 'Cold create to running', 'ms'],
  ['create_first_output_ms', 'Create to first output', 'ms'],
  ['agent_run_true_ms', 'Agent true', 'ms'],
  ['wake_ms', 'Wake from sleep', 'ms'],
  ['snapshot_100mb_ms', 'Snapshot 100 MB', 'ms'],
  ['fork_100mb_ms', 'Fork 100 MB', 'ms'],
  ['idle_context_pss_bytes', 'Idle context PSS', 'MiB'],
  ['idle_increment_pss_bytes', 'Idle paired PSS delta', 'MiB'],
  ['fixed_context_pss_bytes', 'Fixed context PSS before create', 'MiB'],
] as const

function summarize(samples: Samples) {
  const shown = (value: number | undefined, unit: string) =>
    value === undefined ? 'none' : (unit === 'MiB' ? value / (1024 * 1024) : value).toFixed(2)
  console.info('VM SUMMARY cohort: n, p50 / p90')
  for (const [name, label, unit] of FIGURES) {
    const values = samples[name] ?? []
    console.info(
      `VM SUMMARY ${label} (${unit}): n=${values.length}, ${shown(percentile(values, 0.5), unit)} / ${shown(percentile(values, 0.9), unit)}`,
    )
  }
}

async function trueCommand(box: Sandbox) {
  const result = await box.run(['/bin/sh', '-c', 'true'])
  assert.equal(result.code, 0)
  assert.equal(result.stdout, '')
  assert.equal(result.stderr, '')
}

export async function vmMeasurements(sdk: Pryx, installed: LocalInstallation, key: string) {
  const path = process.env.PRYX_VM_MEASUREMENTS
  const socket = process.env.PRYX_VM_MEMORY_SOCKET
  const source = process.env.PRYX_VM_SOURCE
  const ci = process.env.PRYX_VM_CI_URL
  const identity = supervisor(installed)
  assert(path && socket && identity && source && ci)
  const samples: Samples = {
    cold_create_running_ms: [],
    create_first_output_ms: [],
    fixed_context_pss_bytes: [],
    idle_context_pss_bytes: [],
    idle_increment_pss_bytes: [],
  }
  let complete = false
  let operation = 'cold-create'
  const persist = () =>
    writeFileSync(
      path,
      `${JSON.stringify(
        {
          source,
          ci,
          complete,
          operation,
          installed: true,
          isolation: 'vm',
          network: 'internet',
          uid: process.getuid?.(),
          image: inputs[0]?.reference,
          cachedRuntimeAndImage: true,
          datasetBytes: DATA_BYTES,
          repetitions: REPETITIONS,
          host: {
            cpu: cpus()[0]?.model,
            cores: cpus().length,
            memoryBytes: totalmem(),
            kernel: release(),
            bun: Bun.version,
          },
          memory:
            'same supervisor tree PSS including fixed control/edge/node and all VM/packet children; increment subtracts pre-create baseline',
          memorySleep: false,
          samples,
          summary: Object.fromEntries(
            Object.entries(samples).map(([name, values]) => [
              name,
              { n: values.length, p50: percentile(values, 0.5), p90: percentile(values, 0.9) },
            ]),
          ),
        },
        null,
        2,
      )}\n`,
    )
  persist()
  try {
    for (let sample = 0; sample < REPETITIONS; sample++) {
      const baseline = await memory(socket, identity.pid)
      const started = performance.now()
      const box = await make(sdk)
      samples.cold_create_running_ms.push(performance.now() - started)
      try {
        assert.equal(box.isolation, 'vm')
        assert.equal(box.state, 'running')
        const proof = crypto.randomUUID()
        const command = box.stream(['/bin/sh', '-c', 'printf "%s" "$1"', 'vm-output', proof])
        let first: number | undefined
        for await (const chunk of command) {
          if (chunk.stream === 'stdout' && chunk.data && first === undefined) first = performance.now()
        }
        const result = await command.result()
        assert.equal(result.code, 0)
        assert.equal(result.stdout, proof)
        assert.equal(result.stderr, '')
        assert(first !== undefined)
        samples.create_first_output_ms.push(first - started)
        const idle = await memory(socket, identity.pid)
        samples.fixed_context_pss_bytes.push(baseline)
        samples.idle_context_pss_bytes.push(idle)
        samples.idle_increment_pss_bytes.push(idle - baseline)
      } finally {
        assert.equal((await box.delete()).existed, true)
        persist()
      }
    }
    const box = await make(sdk)
    try {
      operation = 'agent-true'
      await trueCommand(box)
      await measure(samples, 'agent_run_true_ms', () => trueCommand(box))
      persist()
      operation = 'cold-wake'
      const proof = crypto.randomUUID()
      await box.write('wake-proof', proof)
      await measure(
        samples,
        'wake_ms',
        async () => {
          await box.wake()
          assert.equal(box.state, 'running')
          return async () => {
            assert.equal(await box.read('wake-proof'), proof)
          }
        },
        async () => {
          await box.sleep()
        },
      )
      persist()
      operation = 'snapshot-fork-100mb'
      await snapshots(box, samples)
    } finally {
      assert.equal((await box.delete()).existed, true)
    }
    for (const values of Object.values(samples)) assert.equal(values.length, REPETITIONS)
    complete = true
    operation = 'complete'
  } finally {
    persist()
    summarize(samples)
  }
  if (process.env.PRYX_MEASURE_VM === 'on') {
    const current = supervisor(installed)
    const port = current?.controlPort
    assert(
      current?.ready === true &&
        current.pid === identity.pid &&
        current.start === identity.start &&
        typeof port === 'number' &&
        Number.isSafeInteger(port) &&
        port > 0,
      'VM creation waves need the installed supervisor’s ready control listener',
    )
    const concurrentSdk = new Pryx({ url: new URL(`http://127.0.0.1:${port}`).origin, key })
    const prefix = join(homedir(), 'pryx-vm-baseline')
    const record = (result: Awaited<ReturnType<typeof collectBaseline>>) =>
      writeBaseline(
        prefix,
        source,
        ci,
        'Signed ordinary-user VM installation with creation waves at its private control listener',
        'Full installed supervisor tree',
        [result],
      )
    const result = await collectBaseline(
      { sdk, concurrentSdk, memory: () => memory(socket, identity.pid) },
      'vm',
      vmRequest(),
      (result) => record({ ...result, complete: false }),
    )
    try {
      await helperMemory(sdk, socket, identity.pid, result.samples)
    } catch (error) {
      result.complete = false
      result.failure = { phase: 'vm_helpers', code: baselineFailureCode(error) }
      throw error
    } finally {
      record(result)
    }
  }
}
