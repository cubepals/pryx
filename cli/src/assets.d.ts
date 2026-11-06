// SPDX-License-Identifier: FSL-1.1-ALv2
/** Bun embeds public release trust anchors in the executable; no runtime key download. */
declare module '*.pem' {
  const text: string
  export default text
}
