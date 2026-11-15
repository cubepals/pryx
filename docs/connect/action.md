---
title: Connect GitHub Actions
---

1. Host pryx at an address your Actions runner can reach; hosted runners cannot reach a private loopback install.
2. Set repository Actions variable `PRYX_API_URL` and secret `PRYX_API_KEY`. Set variable
   `PRYX_IMAGE` to `alpine:3.22` for containers; leave it empty for process or Wasm.
3. Put this step in a workflow job. The Action runs hello and cleans up its sandbox.

```yaml
- uses: cubepals/pryx/action@v0.1.0
  with:
    url: ${{ vars.PRYX_API_URL }}
    key: ${{ secrets.PRYX_API_KEY }}
    image: ${{ vars.PRYX_IMAGE }}
    run: echo hello
```

The tag follows the repository's release version. CI uses this checkout's Action and verified
CLI for unreleased-source acceptance; the existing release tag does not imply it contains
unreleased changes. Images require containers; empty image selects the host default without
substituting a weaker isolation. [Action inputs](../../action/action.yml), [tier boundaries](../tiers.md).
