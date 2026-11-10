// SPDX-License-Identifier: FSL-1.1-ALv2
/** Host artifact names and the release signing statement. Runtime code never has a signing key. */
export const HOST_BUNDLE = 'pryx-bundle-linux-x64.tar.gz'
export const CLI_BOOTSTRAP = 'pryx-bootstrap-linux-x64.tar.gz'
export const bundleStatement = (version: string, digest: string) => {
  if (!/^\d+\.\d+\.\d+$/.test(version) || !/^[a-f0-9]{64}$/.test(digest))
    throw new Error('Invalid host bundle release statement')
  return `pryx host bundle\n${version}\n${digest}\n`
}
