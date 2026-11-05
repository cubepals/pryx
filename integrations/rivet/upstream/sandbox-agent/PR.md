<!-- SPDX-License-Identifier: FSL-1.1-ALv2 -->

Add the pryx deploy target

Add pryx through the current SandboxProvider contract, package subpath, deployment guide and Vitest unit case. A configured project client and stable creation key select a prepared sandbox-agent template; its server is already installed and available through private ingress. Credentials stay in the service environment.

Prepared against public commit `f28965688227b981aae908a88b4a96770cf45e6f`. Applied, typechecked and bundled locally through the prepared-patch checker. The prepared Vitest provider test passes with the version in the public lock (3.2.7 for sandbox-agent, 2.1.9 for agentOS). The agentOS existing boundary test also passes. Real built-agentOS container acceptance passes in pryx CI 42055145022; warmed-template provider acceptance remains pending. The agentOS test requires the dependent prepared sandbox-agent/pryx export, resolved to that public patch source during validation. No upstream pull request, issue or message was sent.
