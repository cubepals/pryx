// SPDX-License-Identifier: FSL-1.1-ALv2
/** Bounded file chunks over the public exec/tar transport. Archives are read in memory, never extracted on the client filesystem. */
import { type Pryx, PryxError } from './api.ts'
import { SDK_DEFAULTS } from './generated/defaults.ts'

function archiveData(encoded: string): Uint8Array {
  const tar = Uint8Array.from(atob(encoded), (c) => c.charCodeAt(0))
  const text = (start: number, length: number) =>
    new TextDecoder()
      .decode(tar.subarray(start, start + length))
      .replace(/\0.*$/s, '')
      .trim()
  const size = Number.parseInt(text(124, 12), 8)
  const checksum = Number.parseInt(text(148, 8), 8)
  let sum = 0
  for (let index = 0; index < 512; index++) sum += index >= 148 && index < 156 ? 32 : (tar[index] ?? 0)
  if (
    text(0, 100) !== 'data' ||
    !['', '0'].includes(text(156, 1)) ||
    !Number.isSafeInteger(size) ||
    size < 0 ||
    size > SDK_DEFAULTS.fileChunkBytes ||
    tar.length < 512 + size ||
    sum !== checksum
  )
    throw new PryxError(0, 'invalid_archive', 'Invalid file chunk archive')
  return tar.slice(512, 512 + size)
}
export class Files {
  constructor(
    private api: Pryx,
    private id: string,
  ) {}
  private path(path: string) {
    return `/v1/sandboxes/${encodeURIComponent(this.id)}/file-chunks/${path.split('/').map(encodeURIComponent).join('/')}`
  }
  async write(path: string, data: string | Uint8Array): Promise<void> {
    const bytes = typeof data === 'string' ? new TextEncoder().encode(data) : data
    for (let offset = 0; offset < bytes.length || offset === 0; offset += SDK_DEFAULTS.fileChunkBytes) {
      const chunk = bytes.subarray(offset, offset + SDK_DEFAULTS.fileChunkBytes)
      const dataBase64 = btoa(Array.from(chunk, (byte) => String.fromCharCode(byte)).join(''))
      await this.api.request(this.path(path), 'PUT', { offset, dataBase64 })
    }
  }
  async readBytes(path: string): Promise<Uint8Array> {
    const chunks: Uint8Array[] = []
    let offset = 0
    while (true) {
      const response = await this.api.request<{ archiveBase64: string }>(
        `${this.path(path)}?offset=${offset}`,
      )
      const chunk = archiveData(response.archiveBase64)
      chunks.push(chunk)
      offset += chunk.length
      if (chunk.length < SDK_DEFAULTS.fileChunkBytes) break
    }
    const result = new Uint8Array(offset)
    let at = 0
    for (const chunk of chunks) {
      result.set(chunk, at)
      at += chunk.length
    }
    return result
  }
  async read(path: string) {
    return new TextDecoder().decode(await this.readBytes(path))
  }
}
