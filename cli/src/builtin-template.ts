// SPDX-License-Identifier: FSL-1.1-ALv2
/** Released language images become ordinary code templates, including their prestarted kernels. */
import type { TemplateOptions } from '@cubepals/pryx'
import { PRYX_VERSION } from '../../control/src/generated/release.ts'
import { KERNEL_DEFAULTS } from '../../sdk/typescript/src/generated/kernels.ts'

export function builtinTemplate(name: string, image?: string): TemplateOptions | undefined {
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
