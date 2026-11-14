// SPDX-License-Identifier: FSL-1.1-ALv2
/** IJavascript speaks Jupyter; current ZeroMQ's official compatibility API serves its legacy
 * jmp binding. Resolution changes only that dependency, never the sandbox user's imports. */
const { registerHooks } = require('node:module')
const { pathToFileURL } = require('node:url')
// NEL uses spawn rather than fork, so NODE_OPTIONS carries the trusted preload to its VM.
const interrupt = require('node:path').join(__dirname, 'interrupt.cjs')
process.env.NODE_OPTIONS = `${process.env.NODE_OPTIONS ?? ''} --require ${JSON.stringify(interrupt)}`
require('./interrupt.cjs')
const compat = require('node:path').join(__dirname, 'legacy-socket.cjs')
const owner = pathToFileURL(require.resolve('jmp')).href
registerHooks({
  resolve(specifier, context, next) {
    return next(specifier === 'zeromq' && context.parentURL === owner ? compat : specifier, context)
  },
})
// jmp subclasses a callable v5 constructor. Create the actual current socket first, then let
// jmp's unmodified constructor initialize its own metadata on that same object.
const zmq = require('./legacy-socket.cjs')
const jmp = require('jmp')
const connection = JSON.parse(require('node:fs').readFileSync(process.argv.at(-1), 'utf8'))
function allocate(type) {
  const socket = zmq.allocate(type)
  if (connection.transport === 'ipc') {
    // jp-kernel fixes TCP syntax. Adapt only its bind address; the maintained ZeroMQ owner
    // still implements IPC, framing and messages, including Linux's abstract-address syntax.
    const prefix = `tcp://${connection.ip}:`
    const bind = socket.bindSync.bind(socket)
    socket.bindSync = address => {
      if (!address.startsWith(prefix)) throw new Error('Unexpected kernel bind address')
      return bind(`ipc://${connection.ip}-${address.slice(prefix.length)}`)
    }
  }
  return socket
}
jmp.zmq.createSocket = allocate
const initialize = jmp.Socket
jmp.Socket = function (type, scheme, key) {
  const socket = allocate(type)
  Object.setPrototypeOf(socket, initialize.prototype)
  initialize.call(socket, type, scheme, key)
  return socket
}
jmp.Socket.prototype = initialize.prototype
const signals = new Set(process.listeners('SIGINT'))
require('ijavascript/lib/kernel')
// The upstream CLI restarts the VM on SIGINT. The inherited VM preload supplies a bounded
// interrupt instead, so retain prior handlers and remove only that newly-installed restart.
for (const listener of process.listeners('SIGINT')) {
  if (!signals.has(listener)) process.removeListener('SIGINT', listener)
}
