// SPDX-License-Identifier: FSL-1.1-ALv2
/** Public tool prerequisites select a tier before compute and are rechecked before wake.
 * Node feature names belong to the Rust contract; this module owns the public choices. */
import contract from '../../openapi/node-writes.openapi.json'

const FEATURES = {
  desktop: contract['x-pryx-native-tools'].desktopFeature,
  ssh: contract['x-pryx-native-tools'].sshFeature,
} as const
export type RequiredTool = keyof typeof FEATURES
export const REQUIRED_TOOLS = Object.keys(FEATURES) as [RequiredTool, ...RequiredTool[]]
export const toolFeature = (tool: RequiredTool) => FEATURES[tool]
export const toolFeatures = (tools: readonly RequiredTool[] = []) => tools.map(toolFeature)

export const TOOL_NETWORK_PROBLEM =
  'Native desktop and SSH are off for network-none; ordinary egress remains denied'
