// SPDX-License-Identifier: FSL-1.1-ALv2
/** Host Docker and firewall prerequisites for installation. Merges the existing daemon policy,
 * reloads an active engine, and never removes existing containers or unrelated settings. */
import { existsSync, readFileSync } from 'node:fs'
import daemonJson from '../../deploy/daemon.json'
import { writeChanged } from './up-files.ts'
export type InstallCommand = (args: string[], quiet?: boolean, redactions?: string[]) => Promise<void>
const DOCKER_INSTALL = `set -eu
. /etc/os-release
case "$ID" in ubuntu|debian) ;; *) exit 1;; esac
apt-get update -q
apt-get install -yq ca-certificates curl
install -d -m 0755 /etc/apt/keyrings
curl -fsSL "https://download.docker.com/linux/$ID/gpg" -o /etc/apt/keyrings/docker.asc
chmod a+r /etc/apt/keyrings/docker.asc
printf 'deb [arch=%s signed-by=/etc/apt/keyrings/docker.asc] https://download.docker.com/linux/%s %s stable\n' "$(dpkg --print-architecture)" "$ID" "$VERSION_CODENAME" > /etc/apt/sources.list.d/docker.list
apt-get update -q
apt-get install -yq docker-ce docker-ce-cli containerd.io
`
export async function configureHostRuntime(run: InstallCommand) {
  if (!Bun.which('docker')) await run(['sh', '-c', DOCKER_INSTALL])
  if (!Bun.which('nft')) {
    await run(['apt-get', 'update', '-q'])
    await run(['apt-get', 'install', '-yq', 'nftables'])
  }
  await run(['systemctl', 'enable', '--now', 'docker'])
  const existing = existsSync('/etc/docker/daemon.json')
    ? JSON.parse(readFileSync('/etc/docker/daemon.json', 'utf8'))
    : {}
  if (
    writeChanged(
      '/etc/docker/daemon.json',
      JSON.stringify({ ...existing, ...daemonJson }, null, 2) + '\n',
      0o644,
    )
  ) {
    const running = Bun.spawnSync(['docker', 'ps', '-q']).stdout.toString().trim()
    if (running) await run(['systemctl', 'reload', 'docker'])
    else await run(['systemctl', 'restart', 'docker'])
  }
}
