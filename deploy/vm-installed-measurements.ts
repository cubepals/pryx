// SPDX-License-Identifier: FSL-1.1-ALv2
/** Thirty real installed VM cohorts. Timed operations use the public API and authenticated agent;
 * idle memory uses the unchanged aggregate process-tree PSS sampler through a CI-only Unix fixture.
 * Raw/partial samples stay in CI artifacts; this records neither credentials nor runtime journals. */
import assert from 'node:assert/strict'
import { writeFileSync } from 'node:fs'
import { createConnection } from 'node:net'
import { cpus, release, totalmem } from 'node:os'
import { type LocalInstallation, supervisor } from '../cli/src/up-local-record.ts'
import { snapshots } from '../control/bench/tiers/operations.ts'
import { DATA_BYTES, measure, percentile, REPETITIONS, type Samples } from '../control/bench/tiers/report.ts'
import { MINUTE_MS, SECOND_MS } from '../control/src/durations.ts'
import type { Pryx, Sandbox } from '../sdk/typescript/src/index.ts'
import inputs from '../templates/vm-tools/image-inputs.json'

async function memory(socket: string, root: number) {
  const observation = await new Promise<string>((resolve, reject) => {
    const client = createConnection(socket)
    let bytes = ''
    client.setTimeout(30 * SECOND_MS, () => client.destroy(new Error('VM PSS observation deadline')))
    client.on('error', reject)
    client.on('connect', () => client.end(JSON.stringify({ root })))
    client.on('data', (part) => {
      bytes += part.toString()
      if (bytes.length > 64 * 1024) client.destroy(new Error('VM PSS observation exceeds its bound'))
    })
    client.on('end', () => resolve(bytes))
  })
  const value = JSON.parse(observation) as { pssBytes?: number; processes?: number }
  assert(typeof value.pssBytes === 'number' && value.pssBytes > 0 && Number.isFinite(value.pssBytes))
  assert(typeof value.processes === 'number' && value.processes >= 1)
  return value.pssBytes
}

function make(sdk: Pryx) {
  assert(inputs[0])
  return sdk.sandbox({
    isolation: 'vm',
    image: inputs[0].reference,
    units: 1,
    idle: (10 * MINUTE_MS) / SECOND_MS,
    sparePorts: 0,
  })
}

async function trueCommand(box: Sandbox) {
  const result = await box.run(['/bin/sh', '-c', 'true'])
  assert.equal(result.code, 0)
  assert.equal(result.stdout, '')
  assert.equal(result.stderr, '')
}

export async function vmMeasurements(sdk: Pryx, installed: LocalInstallation) {
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
  }
}
