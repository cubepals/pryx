// SPDX-License-Identifier: FSL-1.1-ALv2
/** Identify the supervisor across CLI invocations without trusting a reused process id. */
import { readFileSync } from 'node:fs'

export function processStart(pid: number): string | undefined {
  try {
    const stat = readFileSync(`/proc/${pid}/stat`, 'utf8')
    // proc_pid_stat(5): starttime is field 22; field 3 follows the final comm parenthesis.
    return stat.slice(stat.lastIndexOf(')') + 2).split(/\s+/)[19]
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === 'ENOENT') return undefined
    throw error
  }
}
