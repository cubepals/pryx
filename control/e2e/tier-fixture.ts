// SPDX-License-Identifier: FSL-1.1-ALv2
/** Equivalent software fixtures; all K11 agent assertions and SDK transports remain shared. */
export function webCommand(): string[] {
  return process.env.PRYX_TEST_TIER === 'wasm'
    ? [
        'node',
        '-e',
        "require('node:http').createServer((req,res)=>res.end(require('node:fs').readFileSync('/data/index.html'))).listen(3000,'0.0.0.0')",
      ]
    : ['httpd', '-f', '-p', '3000', '-h', '/data']
}
