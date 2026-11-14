// SPDX-License-Identifier: FSL-1.1-ALv2
/** Node's HTTP parser decodes streamed requests before this preview fixture echoes their body. */
const { createServer } = require('node:http')
const { existsSync, readFileSync } = require('node:fs')

module.exports = () =>
  createServer(async (request, response) => {
    if (new URL(request.url, 'http://preview').pathname === '/echo') {
      const chunks = []
      for await (const chunk of request) chunks.push(chunk)
      response.end(
        `${request.headers.host}:${request.method}:${request.url}:${Buffer.concat(chunks).toString()}`,
      )
    } else {
      const index = '/data/index.html'
      response.end(existsSync(index) ? readFileSync(index) : '')
    }
  })

if (require.main === module) module.exports().listen(Number(process.argv[2]), process.argv[3])
