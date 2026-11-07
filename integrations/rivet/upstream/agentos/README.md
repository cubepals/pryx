<!-- SPDX-License-Identifier: FSL-1.1-ALv2 -->

# Prepared agentos patch

Base: `4662e362fb93adcfbfc86bfd51b51855d463f371`. Apply `change.patch` with `git apply --check` followed by `git apply` in a clean checkout. This patch adds the existing external sandboxAgentProvider process and filesystem boundary. No upstream submission has been made.

Prepared against public commit `4662e362fb93adcfbfc86bfd51b51855d463f371`. The prepared-patch checker applies both dependent patches, typechecks their added modules and bundles them against those public sources. Local validation passes the new agentOS provider and existing boundary cases with the public lock's Vitest 2.1.9, and the sandbox-agent provider case with its public lock's Vitest 3.2.7. Ephemeral test dependencies are used, and sandbox-agent/pryx resolves to the dependent prepared provider source. CI runs real container acceptance through the built public adapters. Nothing was submitted upstream.
