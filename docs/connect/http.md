---
title: Connect with plain HTTP
---

1. Open welcome and retrieve the active project key from the stated location.
2. Set `PRYX_API_URL` and `PRYX_API_KEY` in your shell's private environment.
3. Run this to read your project and the install's default tier.

```sh
curl --fail --silent --show-error "${PRYX_API_URL%/}/v1/project" \
  --header "Authorization: Bearer $PRYX_API_KEY"
```

This reads configuration; it does not create a sandbox. Create requests must respect the
reported tier: images need containers, while process and Wasm refuse them. The
[API reference](../api.md) describes lifecycle operations, capabilities, errors and webhook signing.
