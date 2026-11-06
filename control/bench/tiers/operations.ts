// SPDX-License-Identifier: FSL-1.1-ALv2
/** Validated agent latency, retained files, package installations and concurrent admission. */
import type { MakeRequest, Sandbox } from '../../../sdk/typescript/src/index.ts'
import { TIER_BENCH_LARGE_UNIT, TIER_BENCH_LARGE_UNITS, TIER_BENCH_UNIT } from '../../e2e/runtime-config.ts'
import type { Isolation } from '../../src/isolation.ts'
import type { Context } from './context.ts'
import { DATA_BYTES, measure, REPETITIONS, type Samples } from './report.ts'

export function creation(tier: Isolation): MakeRequest {
  return {
    isolation: tier,
    ...(tier === 'container' ? { image: process.env.PRYX_IMAGE_TEST ?? 'pryx-static-test' } : {}),
    resources: TIER_BENCH_UNIT,
    sparePorts: 0,
  }
}
export async function cold(c: Context, tier: Isolation, samples: Samples) {
  let baseline = await c.memory()
  samples.fixed_node_pss_bytes = [baseline]
  samples.idle_increment_pss_bytes = []
  samples.shell_increment_pss_bytes = []
  await measure(samples, 'cold_create_running_ms', async () => {
    const box = await c.sdk.sandbox(creation(tier))
    if (box.state !== 'running' || box.isolation !== tier || box.units !== 1)
      throw new Error('Cold sandbox differs')
    return async () => {
      samples.idle_increment_pss_bytes?.push((await c.memory()) - baseline)
      const marker = 'shell-ready'
      const shell = await box.start(['sh', '-c', `printf ${marker}; read value`], {
        stdin: true,
        timeout: '30s',
      })
      let greeting = ''
      for await (const chunk of shell.stream()) {
        greeting += chunk.data
        if (!marker.startsWith(greeting)) throw new Error('Shell readiness output differs')
        if (greeting === marker) break
      }
      if (greeting !== marker) throw new Error('Shell did not become ready')
      samples.shell_increment_pss_bytes?.push((await c.memory()) - baseline)
      await shell.stdin.write('done\n')
      if ((await shell.wait()).code !== 0) throw new Error('Shell failed')
      await box.delete()
      baseline = await c.memory()
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
export async function packages(c: Context, tier: Isolation, box: Sandbox, samples: Samples) {
  await box.files.delete('payload')
  for (const [name, program, install] of [
    [
      'pip',
      'pip3',
      ['install', '--disable-pip-version-check', '--no-cache-dir', 'peppercorn==0.6', '--target'],
    ],
    ['npm', 'npm', ['install', '--ignore-scripts', '--no-audit', '--no-fund', 'is-number@7.0.0', '--prefix']],
  ] as const) {
    const probe = await box.run([program, '--version'])
    if (probe.code !== 0)
      throw new Error(`${program} availability failed (${probe.code}): ${probe.stderr.slice(-2048)}`)
    let pending: Sandbox | undefined
    try {
      await measure(
        samples,
        `${name}_install_ms`,
        async () => {
          if (!pending) throw new Error('Package sandbox missing')
          const fresh = pending,
            target = `k14-${name}`
          const result = await fresh.run(
            tier === 'wasm' && name === 'pip'
              ? [program, 'install', 'peppercorn==0.6']
              : [program, ...install, target, ...(name === 'npm' ? ['--cache', `${target}/.cache`] : [])],
            { timeout: '2m' },
          )
          if (result.code !== 0) throw new Error(`${name} installation failed: ${result.stderr.slice(-2048)}`)
          return async () => {
            pending = undefined
            try {
              const verify =
                name === 'pip'
                  ? [
                      'python3',
                      '-c',
                      `import sys, pathlib; sys.path.insert(0, '${target}'); import peppercorn, importlib.metadata; assert importlib.metadata.version('peppercorn') == '0.6'; assert peppercorn.parse([('name', 'hello')]) == {'name': 'hello'}${tier === 'wasm' ? '' : `; assert pathlib.Path(peppercorn.__file__).resolve().is_relative_to(pathlib.Path('${target}').resolve())`}`,
                    ]
                  : [
                      'node',
                      '-e',
                      `if(!require('./${target}/node_modules/is-number')(42)) throw Error('package differs')`,
                    ]
              const verified = await fresh.run(verify)
              if (verified.code !== 0)
                throw new Error(`${name} package verification failed: ${verified.stderr.slice(-2048)}`)
            } finally {
              await fresh.delete()
            }
          }
        },
        async () => {
          pending = await c.sdk.sandbox({ ...creation(tier), resources: TIER_BENCH_LARGE_UNIT })
          if (pending.units !== TIER_BENCH_LARGE_UNITS) throw new Error('Package unit envelope differs')
          if (tier === 'wasm' && name === 'pip') {
            const absent = await pending.run([
              'python3',
              '-c',
              "import importlib.util; assert importlib.util.find_spec('peppercorn') is None",
            ])
            if (absent.code !== 0) throw new Error('Package was already available before installation')
          }
        },
      )
    } finally {
      await pending?.delete()
    }
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
    if (
      failure ||
      boxes.some((box) => box.state !== 'running' || box.isolation !== tier || box.units !== 1)
    ) {
      await Promise.all(boxes.map((box) => box.delete()))
      throw failure?.status === 'rejected' ? failure.reason : new Error('Concurrent sandbox did not run')
    }
    samples.concurrent_50_running_ms.push(performance.now() - start)
    try {
      const used = await c.memory()
      samples.concurrent_50_pss_bytes.push(used)
      samples.concurrent_50_increment_pss_bytes.push(used - baseline)
      console.info('CONCURRENT_50', tier, i + 1, samples.concurrent_50_running_ms.at(-1), used)
    } finally {
      await Promise.all(boxes.map((box) => box.delete()))
    }
  }
}
