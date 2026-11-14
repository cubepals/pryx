// SPDX-License-Identifier: FSL-1.1-ALv2
/** Recognize container installations before using local engine bind paths and published ports.
 * An engine socket mounted into a container does not establish a shared host filesystem. */
import { existsSync } from 'node:fs'

export function containerContext() {
  return (
    existsSync('/.dockerenv') ||
    existsSync('/run/.containerenv') ||
    existsSync('/run/systemd/container') ||
    Boolean(process.env.container)
  )
}
