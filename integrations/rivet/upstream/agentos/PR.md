<!-- SPDX-License-Identifier: FSL-1.1-ALv2 -->

Add the pryx sandbox provider

Add pryx through the existing external sandboxAgentProvider process and filesystem boundary. A configured project client and stable creation key select a prepared sandbox-agent template; its server is already installed and available through private ingress. Credentials stay in the service environment.

Prepared against public commit `785e6441b7fb511857f28e3b2e11f2f655165d58`. Applied, typechecked and bundled locally through the prepared-patch checker. The prepared Vitest provider test passes with the version in the public lock (3.2.7 for sandbox-agent, 2.1.9 for agentOS). The agentOS existing boundary test also passes. Real built-agentOS container acceptance passes in pryx CI 42055145022; CI 42066544946 passes warmed-template acceptance through the built public provider. The agentOS test requires the dependent prepared sandbox-agent/pryx export, resolved to that public patch source during validation. No upstream pull request, issue or message was sent.
