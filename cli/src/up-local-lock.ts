// SPDX-License-Identifier: FSL-1.1-ALv2
/** Serialize lifecycle mutations with the embedded SQLite engine. This is a mutex, never a
 * store for deployment data, and its transaction ends before following a foreground service. */
import { Database, SQLiteError, constants as sqlite } from 'bun:sqlite'
import { closeSync, constants, fstatSync, openSync } from 'node:fs'

// SQLite result-code name (https://sqlite.org/rescode.html#busy); Bun exports open flags,
// but exposes result names through SQLiteError.code rather than named result constants.
const SQLITE_BUSY = 'SQLITE_BUSY'
// SQLite closes an unreachable Database during GC. Keep a strong reference until the
// explicit release, including callers that keep only the ownership effect.
const held = new Map<string, Database>()
export function acquireLifecycle(path: string) {
  if (held.has(path)) throw new Error('Another pryx lifecycle operation is active')
  const fd = openSync(
    path,
    constants.O_RDWR | constants.O_CREAT | constants.O_NOFOLLOW | constants.O_NONBLOCK,
    0o600,
  )
  let database: Database | undefined
  try {
    const info = fstatSync(fd)
    if (!info.isFile() || info.uid !== process.getuid?.() || info.mode & 0o077 || info.nlink !== 1)
      throw new Error('Lifecycle lock must be an owner-only regular file with one link')
    database = new Database(path, sqlite.SQLITE_OPEN_READWRITE | sqlite.SQLITE_OPEN_NOFOLLOW)
    database.exec('PRAGMA busy_timeout=0; PRAGMA journal_mode=MEMORY; BEGIN EXCLUSIVE')
    held.set(path, database)
  } catch (error) {
    database?.close()
    closeSync(fd)
    if (error instanceof SQLiteError && error.code === SQLITE_BUSY)
      throw new Error('Another pryx lifecycle operation is active; let it finish before retrying')
    throw error
  }
  let released = false
  return () => {
    if (released) return
    released = true
    try {
      database.exec('ROLLBACK')
    } finally {
      database.close()
      closeSync(fd)
      held.delete(path)
    }
  }
}
