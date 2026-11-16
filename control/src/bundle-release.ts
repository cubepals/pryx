// SPDX-License-Identifier: FSL-1.1-ALv2
/** Host artifact names and the release signing statement. Runtime code never has a signing key. */
/** The machines a release is built for, by Node's name (process.arch, which asset names use) and
 * the kernel's (`uname -m`, which a bundle's metadata and the host probe use). */
export const RELEASE_MACHINES: Readonly<Record<string, string>> = { x64: 'x86_64', arm64: 'aarch64' }
/** This host's machine as `uname -m` names it: what its bundle is built for. */
export const HOST_MACHINE = RELEASE_MACHINES[process.arch] ?? process.arch
export const releasedFor = (machine: string) => Object.values(RELEASE_MACHINES).includes(machine)
/** A release file built for this host's machine: x86_64 keeps the first release's plain name, and
 * any other machine's file has the machine after its stem (pryxd-linux-arm64.sig). */
export const releaseAsset = (name: string, arch: string = process.arch) =>
  arch === 'x64' ? name : name.replace(/^[^.]+/, (stem) => `${stem}-linux-${arch}`)
export const HOST_BUNDLE = `pryx-bundle-linux-${process.arch}.tar.gz`
export const CLI_BOOTSTRAP = `pryx-bootstrap-linux-${process.arch}.tar.gz`
export const bundleStatement = (version: string, digest: string) => {
  if (!/^\d+\.\d+\.\d+$/.test(version) || !/^[a-f0-9]{64}$/.test(digest))
    throw new Error('Invalid host bundle release statement')
  return `pryx host bundle\n${version}\n${digest}\n`
}
