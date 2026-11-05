<!-- SPDX-License-Identifier: FSL-1.1-ALv2 -->

# Prepared sandbox-agent patch

Base: `f28965688227b981aae908a88b4a96770cf45e6f`. Apply `change.patch` with `git apply --check` followed by `git apply` in a clean checkout. This patch adds the current SandboxProvider contract, package subpath, deployment guide and Vitest unit case. No upstream submission has been made.

Local validation: `PRYX_PATCH_SOURCE_ROOT=/workspace bun integrations/rivet/upstream/check.ts /tmp/pryx-rivet-public-patches` applied both patches, typechecked both added modules against these public sources, and bundled both modules successfully. The prepared standalone Vitest case has not run locally. CI builds these same modules and uses them in real sandbox acceptance; that result remains pending.
