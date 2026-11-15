---
title: Connect Python
---

1. Install `cubepals-pryx` in your application's Python environment.
2. Set `PRYX_API_URL` and `PRYX_API_KEY` in that process's private environment.
3. Run this with the same Python interpreter.

```python
import os
from cubepals_pryx import Pryx
with Pryx(url=os.environ['PRYX_API_URL'], key=os.environ['PRYX_API_KEY']) as pryx:
    defaults = pryx.project()['defaults']
    options = {'image': 'alpine:3.22'} if defaults['isolation'] == 'container' else {}
    box = pryx.sandbox(isolation=defaults['isolation'], **options)
    try:
        result = box.run('echo hello')
        print(result.stdout)
        assert result.code == 0
    finally:
        box.delete()
```

This names the reported default tier and adds an image only for containers. An unavailable
named tier is refused. See [tier limits](../tiers.md) and the
[sync and async SDK reference](../../sdk/python/README.md).
