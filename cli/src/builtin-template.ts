// SPDX-License-Identifier: FSL-1.1-ALv2
/** Released language images become ordinary code templates, including their prestarted kernels. */
import type { TemplateOptions } from '@cubepals/pryx'
import { PRYX_VERSION } from '../../control/src/generated/release.ts'
import { KERNEL_DEFAULTS } from '../../sdk/typescript/src/generated/kernels.ts'
import { TOOL_DEFAULTS } from '../../sdk/typescript/src/generated/tools.ts'

export function builtinTemplate(name: string, image?: string): TemplateOptions | undefined {
  if (['desktop', 'browser', 'tools'].includes(name))
    return {
      name,
      from: image ?? `ghcr.io/cubepals/pryx-${name}:${PRYX_VERSION}`,
      isolation: 'container',
      sparePorts: 3,
      env: { [TOOL_DEFAULTS.kindEnv]: name },
      ready:
        name === 'desktop'
          ? 'command -v Xvfb && command -v xdotool'
          : name === 'browser'
            ? 'node -e "require(\'/opt/pryx/browser/node_modules/playwright-core\')"'
            : 'command -v sshd && command -v rclone',
      start:
        name === 'desktop'
          ? 'exec python3 /opt/pryx/desktop/start.py'
          : name === 'browser'
            ? 'exec node /opt/pryx/browser/start.cjs'
            : 'exec sleep infinity',
    }
  const language =
    name === 'python-interpreter' ? 'python' : name === 'node-interpreter' ? 'javascript' : undefined
  if (!language) return
  return {
    name,
    from: image ?? `ghcr.io/cubepals/pryx-${name}:${PRYX_VERSION}`,
    isolation: 'container',
    env: { [KERNEL_DEFAULTS.prestartEnv]: language },
    ready:
      language === 'python'
        ? 'python3 -c "import ipykernel,numpy,pandas,matplotlib,scipy,sklearn,requests,PIL"'
        : 'node -e "require(\'/opt/pryx/node_modules/ijavascript/package.json\')"',
    start: 'exec sleep infinity',
  }
}
