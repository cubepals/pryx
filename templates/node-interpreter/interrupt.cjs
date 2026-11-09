// SPDX-License-Identifier: FSL-1.1-ALv2
/** Node's documented breakOnSigint interrupts the NEL VM without killing its state.
 * NEL inherits this preload through fork; idle SIGINT has no destructive default handler. */
const vm = require('node:vm')
const run = vm.runInThisContext
vm.runInThisContext = function (code, options) {
  return run.call(this, code, { ...(typeof options === 'string' ? { filename: options } : options), breakOnSigint: true })
}
process.on('SIGINT', () => {})
