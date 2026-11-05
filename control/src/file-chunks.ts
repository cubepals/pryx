// SPDX-License-Identifier: FSL-1.1-ALv2
/** Additive chunk transport inside containers. Each request is bounded, paths stay relative to persistent storage, and no client archive is unpacked on the host. */
import { z } from 'zod'
import type { SandboxIO } from './sandbox-io.ts'
import { SandboxError } from './sandboxes.ts'
import { SDK_DEFAULTS } from './sdk-defaults.ts'

const bytes = SDK_DEFAULTS.fileChunkBytes
export const chunkOffset = z.coerce
  .number()
  .int()
  .nonnegative()
  .max(Number.MAX_SAFE_INTEGER - bytes)
  .refine((n) => n % bytes === 0, 'Offset must be a chunk boundary')
export const chunkWrite = z
  .object({
    offset: chunkOffset,
    dataBase64: z
      .string()
      .max(Math.ceil(bytes / 3) * 4)
      .regex(/^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/),
  })
  .strict()
export class FileChunks {
  constructor(private io: SandboxIO) {}
  async transfer(id: string, project: string, path: string, offset: number, data?: string) {
    const row = await this.io.boxes.record(id, project)
    if (
      !path ||
      path.startsWith('/') ||
      path.includes('\0') ||
      path.split('/').some((p) => !p || p === '.' || p === '..')
    )
      throw new SandboxError('invalid_request', 'File paths are relative without dot segments')
    if (data !== undefined && Buffer.from(data, 'base64').length > bytes)
      throw new SandboxError('invalid_request', 'File chunk exceeds the transfer bound')
    // All substitutions are inside the sandbox's shell; path and payload are argv, never source.
    const guard =
      `base=$(realpath "$1") || exit; rel="$2"; parent="$base"; while [ "\${rel#*/}" != "$rel" ]; do part=\${rel%%/*}; rel=\${rel#*/}; parent="$parent/$part"; [ ! -L "$parent" ] || exit 1; ` +
      (data === undefined ? '[ -d "$parent" ] || exit 1; ' : 'mkdir -p -- "$parent" || exit; ') +
      'done; target="$parent/$rel"; [ ! -L "$target" ] || exit 1; '
    const script =
      data === undefined
        ? `${guard} [ -f "$target" ] || exit 1; tmp=$(mktemp -d) || exit; trap 'rm -rf -- "$tmp"' EXIT; dd if="$target" of="$tmp/data" bs=${bytes} skip="$3" count=1 2>/dev/null || exit; tar -C "$tmp" -cf - data | base64`
        : `${guard} if [ "$3" = 0 ]; then : > "$target" || exit; else [ -f "$target" ] || exit 1; fi; printf %s "$4" | base64 -d | dd of="$target" bs=${bytes} seek="$3" conv=notrunc 2>/dev/null`
    const result = await this.io.exec(id, project, {
      command: [
        'sh',
        '-c',
        script,
        'pryx-chunk',
        row.spec.storage.mountPath,
        path,
        String(offset / bytes),
        ...(data === undefined ? [] : [data]),
      ],
      timeout: 30,
      env: {},
    })
    if (result.exitCode !== 0 || result.stdoutTruncated || result.stderrTruncated)
      throw new SandboxError(
        'invalid_request',
        'File chunk failed; image needs sh, realpath, mkdir, mktemp, dd, tar and base64',
      )
    return data === undefined ? { archiveBase64: result.stdout.replaceAll(/\s/g, '') } : { written: true }
  }
}
