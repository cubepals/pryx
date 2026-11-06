// SPDX-License-Identifier: FSL-1.1-ALv2
/** Raw same-host cohorts. Failed samples remain evidence; percentiles use nearest ranks. */
import { writeFileSync } from 'node:fs'
import { cpus, release, totalmem } from 'node:os'

export const REPETITIONS = 30
export const DATA_BYTES = 100_000_000
export type Samples = Record<string, number[]>
export function percentile(values: number[], fraction: number) {
  return [...values].sort((a, b) => a - b)[Math.ceil(values.length * fraction) - 1]
}
export function report(context: string, samples: Samples, unsupported: Record<string, string>) {
  return {
    context,
    source: Bun.spawnSync(['git', 'rev-parse', 'HEAD']).stdout.toString().trim(),
    comparison: 'same-source container/process reference versus wasm candidate; release profile',
    host: {
      cpu: cpus()[0]?.model,
      cores: cpus().length,
      kernel: release(),
      memoryBytes: totalmem(),
      bun: Bun.version,
    },
    datasetBytes: DATA_BYTES,
    repetitions: REPETITIONS,
    samples,
    unsupported,
  }
}
export function save(path: string, value: ReturnType<typeof report>, extra: object) {
  writeFileSync(
    path,
    `${JSON.stringify(
      {
        ...value,
        ...extra,
        summary: Object.fromEntries(
          Object.entries(value.samples).map(([name, values]) => [
            name,
            { n: values.length, p50: percentile(values, 0.5), p90: percentile(values, 0.9) },
          ]),
        ),
      },
      null,
      2,
    )}\n`,
  )
}
export async function command(args: string[], env = process.env) {
  const child = Bun.spawn(args, { env, stdout: 'pipe', stderr: 'pipe' })
  const [stdout, stderr, code] = await Promise.all([
    new Response(child.stdout).text(),
    new Response(child.stderr).text(),
    child.exited,
  ])
  if (code) throw new Error(`${args[0]} exited ${code}: ${stderr.slice(-2048)}`)
  return stdout
}
export async function measure(
  samples: Samples,
  name: string,
  operation: () => Promise<void | (() => Promise<void>)>,
  prepare?: () => Promise<void>,
) {
  const values = samples[name] ?? []
  samples[name] = values
  for (let i = 0; i < REPETITIONS; i++) {
    if (prepare) await prepare()
    const start = performance.now()
    const cleanup = await operation()
    values.push(performance.now() - start)
    if (cleanup) await cleanup()
  }
  console.info('METRIC', name, values.length, 'p50', percentile(values, 0.5), 'p90', percentile(values, 0.9))
}
