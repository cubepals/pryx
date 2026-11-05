// SPDX-License-Identifier: FSL-1.1-ALv2
/** Validated agent latency, retained files, package installations and concurrent admission. */
import type { MakeRequest, Sandbox } from '../../../sdk/typescript/src/index.ts'
import type { Isolation } from '../../src/isolation.ts'
import type { Context } from './context.ts'
import { DATA_BYTES, measure, REPETITIONS, type Samples } from './report.ts'

export function creation(tier: Isolation): MakeRequest {
  return {
    isolation: tier,
    ...(tier === 'container' ? { image: process.env.PRYX_IMAGE_TEST ?? 'pryx-static-test' } : {}),
    resources: { memoryMb: 64, cpuMillis: 10 },
    sparePorts: 0,
  }
}
export async function cold(c: Context, tier: Isolation, samples: Samples) {
  const baseline = await c.memory()
  samples.fixed_node_pss_bytes = [baseline]
  samples.idle_increment_pss_bytes = []
  samples.shell_increment_pss_bytes = []
  await measure(samples, 'cold_create_running_ms', async () => {
    const box = await c.sdk.sandbox(creation(tier))
    if (box.state !== 'running' || box.isolation !== tier) throw new Error('Cold sandbox differs')
    return async () => {
      samples.idle_increment_pss_bytes?.push((await c.memory()) - baseline)
      const shell = await box.start(['sh', '-c', 'read value'], { stdin: true })
      samples.shell_increment_pss_bytes?.push((await c.memory()) - baseline)
      await shell.stdin.write('done\n')
      if ((await shell.wait()).code !== 0) throw new Error('Shell failed')
      await box.delete()
    }
  })
}
export async function hot(box: Sandbox, samples: Samples) {
  if ((await box.run('true')).code !== 0) throw new Error('Agent warmup failed')
  await measure(samples, 'agent_run_true_ms', async () => {
    if ((await box.run('true')).code !== 0) throw new Error('true failed')
  })
  samples.first_byte_echo_ms = []
  for (let i = 0; i < REPETITIONS; i++) {
    const start = performance.now(),
      run = box.run('echo hi'),
      stream = run.stream()
    if (!(await stream.next()).value?.data) throw new Error('Echo emitted no first byte')
    samples.first_byte_echo_ms.push(performance.now() - start)
    while (!(await stream.next()).done) {}
    const result = await run
    if (result.code !== 0 || result.stdout !== 'hi\n' || result.stderr !== '') throw new Error('Echo failed')
  }
  await box.write('sleep-marker', 'persisted')
  await measure(samples, 'sleep_then_wake_ms', async () => {
    await box.sleep()
    await box.wake()
    return async () => {
      if ((await box.read('sleep-marker')) !== 'persisted') throw new Error('Sleep lost files')
    }
  })
}
export async function snapshots(box: Sandbox, samples: Samples) {
  await box.write(
    'payload',
    (async function* () {
      const chunk = new Uint8Array(1024 * 1024).fill(42)
      for (let left = DATA_BYTES; left > 0; left -= chunk.length)
        yield chunk.subarray(0, Math.min(left, chunk.length))
    })(),
  )
  if ((await box.files.stat('payload')).size !== DATA_BYTES) throw new Error('Snapshot dataset differs')
  await measure(samples, 'snapshot_100mb_ms', async () => {
    const snapshot = await box.snapshot()
    return async () => {
      await snapshot.delete()
    }
  })
  const snapshot = await box.snapshot()
  try {
    await measure(samples, 'fork_100mb_ms', async () => {
      const fork = await snapshot.fork()
      return async () => {
        try {
          if ((await fork.files.stat('payload')).size !== DATA_BYTES) throw new Error('Fork lost data')
          if ((await fork.run('true')).code !== 0) throw new Error('Fork failed to run')
          await fork.files.delete('payload')
        } finally {
          await fork.delete()
        }
      }
    })
  } finally {
    await snapshot.delete()
  }
}
export async function packages(box: Sandbox, samples: Samples, unsupported: Record<string, string>) {
  await box.files.delete('payload')
  for (const [name, check, install] of [
    ['pip', 'pip3 --version', 'pip3 install --disable-pip-version-check --no-cache-dir six==1.17.0 --target'],
    ['npm', 'npm --version', 'npm install --ignore-scripts --no-audit --no-fund is-number@7.0.0 --prefix'],
  ]) {
    if (!name || !check || !install) throw new Error('Package cohort definition missing')
    const probe = await box.run(check)
    if (probe.code !== 0) {
      unsupported[`${name}_install_ms`] = `command unavailable: ${probe.stderr.slice(0, 512)}; n=0`
      continue
    }
    let index = 0
    await measure(samples, `${name}_install_ms`, async () => {
      const target = `k14-${name}-${index++}`
      const result = await box.run(
        `${install} ${target}${name === 'npm' ? ` --cache ${target}/.cache` : ''}`,
        { timeout: '2m' },
      )
      if (result.code !== 0) throw new Error(`${name} installation failed: ${result.stderr.slice(-2048)}`)
      return async () => {
        const verify =
          name === 'pip'
            ? [
                'python3',
                '-c',
                `import sys; sys.path.insert(0, '${target}'); import six; assert six.__version__ == '1.17.0'`,
              ]
            : [
                'node',
                '-e',
                `if(!require('./${target}/node_modules/is-number')(42)) throw Error('package differs')`,
              ]
        if ((await box.run(verify)).code !== 0) throw new Error(`${name} package verification failed`)
      }
    })
  }
}
export async function concurrent(c: Context, tier: Isolation, samples: Samples) {
  samples.concurrent_50_increment_pss_bytes = []
  samples.concurrent_50_running_ms = []
  samples.concurrent_50_pss_bytes = []
  for (let i = 0; i < REPETITIONS; i++) {
    const baseline = await c.memory()
    const start = performance.now()
    const made = await Promise.allSettled(Array.from({ length: 50 }, () => c.sdk.sandbox(creation(tier))))
    const boxes = made.flatMap((result) => (result.status === 'fulfilled' ? [result.value] : []))
    const failure = made.find((result) => result.status === 'rejected')
    if (failure || boxes.some((box) => box.state !== 'running')) {
      await Promise.all(boxes.map((box) => box.delete()))
      throw failure?.status === 'rejected' ? failure.reason : new Error('Concurrent sandbox did not run')
    }
    samples.concurrent_50_running_ms.push(performance.now() - start)
    try {
      const used = await c.memory()
      samples.concurrent_50_pss_bytes.push(used)
      samples.concurrent_50_increment_pss_bytes.push(used - baseline)
    } finally {
      await Promise.all(boxes.map((box) => box.delete()))
    }
  }
}
