<!-- SPDX-License-Identifier: FSL-1.1-ALv2 -->

# Prepared agentos patch

Base: `785e6441b7fb511857f28e3b2e11f2f655165d58`. Apply `change.patch` with `git apply --check` followed by `git apply` in a clean checkout. This patch adds the existing external sandboxAgentProvider process and filesystem boundary. No upstream submission has been made.

Local validation: `PRYX_PATCH_SOURCE_ROOT=/workspace bun integrations/rivet/upstream/check.ts /tmp/pryx-rivet-public-patches` applied both patches, typechecked both added modules against these public sources, and bundled both modules successfully. The prepared standalone Vitest case has not run locally. CI builds these same modules and uses them in real sandbox acceptance; that result remains pending.
