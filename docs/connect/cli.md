---
title: Connect the CLI
---

1. Install the CLI with the [one-command installer](../index.md#one-command-to-install-the-cli).
2. Export `PRYX_API_URL` and `PRYX_API_KEY` privately. If welcome reports `container`, also
   export `PRYX_IMAGE=alpine:3.22`; leave `PRYX_IMAGE` unset for process or Wasm.
3. Run this block. It prints hello and removes its sandbox.

```sh
set -eu
set --
if [ -n "${PRYX_IMAGE:-}" ]; then set -- --image "$PRYX_IMAGE"; fi
pryx run "$@" -- echo hello
```

Images require the container tier and its engine. Process and Wasm refuse them; the block
keeps that refusal. Welcome supplies an exact block for its configured tier.
See [host boundaries](../hosting.md#tiers-and-guarantees).
