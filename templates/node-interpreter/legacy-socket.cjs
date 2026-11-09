// SPDX-License-Identifier: FSL-1.1-ALv2
/** jmp's callable constructor initializes only its metadata; kernel.cjs already allocated
 * the actual official v5-compat socket before invoking that upstream initializer. */
const compat = require('zeromq/v5-compat')
function InitializedSocket() {}
InitializedSocket.prototype = compat.Socket.prototype
function allocate(type) {
  const socket = new compat.Socket(type)
  // Old ZeroMQ accepted identity on PUB/REP, where it has no routing role. Current ZeroMQ
  // correctly has no routingId property on them; preserve that irrelevant legacy assignment.
  if (type === 'pub' || type === 'rep') Object.defineProperty(socket, 'identity', { set() {} })
  return socket
}
module.exports = { ...compat, Socket: InitializedSocket, createSocket: allocate, allocate }
