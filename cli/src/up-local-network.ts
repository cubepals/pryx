// SPDX-License-Identifier: FSL-1.1-ALv2
/** Choose a free loopback gateway port; an explicit port is either honored or refused. */
import { createServer } from 'node:net'

async function available(port: number) {
  const server = createServer()
  await new Promise<void>((resolve, reject) => {
    server.once('error', reject)
    server.listen(port, '127.0.0.1', resolve)
  })
  const address = server.address()
  await new Promise<void>((resolve) => server.close(() => resolve()))
  if (!address || typeof address === 'string') throw new Error('Could not allocate a loopback listener')
  return address.port
}
export function configuredPort() {
  if (process.env.PORT === undefined) return
  const port = Number(process.env.PORT)
  if (!Number.isInteger(port) || port <= 1024 || port > 65535)
    throw new Error('The local gateway PORT must be an integer above 1024 and at most 65535')
  return port
}
export async function localPort(explicit?: number) {
  if (explicit !== undefined) return await available(explicit)
  try {
    return await available(7331)
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== 'EADDRINUSE') throw error
    return await available(0)
  }
}
