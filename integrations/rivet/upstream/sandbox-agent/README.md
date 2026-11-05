<!-- SPDX-License-Identifier: FSL-1.1-ALv2 -->

# Prepared sandbox-agent patch

Base: `f28965688227b981aae908a88b4a96770cf45e6f`. Apply `change.patch` with `git apply --check` followed by `git apply` in a clean checkout. This patch adds the current SandboxProvider contract, package subpath, deployment guide and Vitest unit case. No upstream submission has been made.

Local validation: the prepared-patch checker applied both patches, typechecked both added modules and bundled them against the exact public sources. The sandbox-agent provider Vitest case passes with its public lock's Vitest 3.2.7; the new agentOS case and its existing provider-boundary case pass with that project's Vitest 2.1.9. Ephemeral test dependencies were used. The agentOS test resolves sandbox-agent/pryx to the prepared provider source until that dependent upstream export is released. CI 42055145022 passes real container acceptance through the built agentOS patch; CI 42066544946 also passes the warmed sandbox-agent template through the built public provider. No upstream submission occurred.
