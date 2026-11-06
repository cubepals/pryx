### Three-tier concurrent capacity

Every wave requests all 50 sandboxes, for 30 waves per tier in CI. Capacity refusal is a measured outcome. Only complete waves enter full-50 latency percentiles.
Memory is total context PSS (including fixed services) after all requests settle and before cleanup; delta subtracts the pre-wave baseline. It is not per-sandbox memory.
A no_room response records observed API admission refusal, not a proven physical memory ceiling. Other creation, validation, memory and cleanup failures still fail the job.

#### local one-box/wasm capacity validation — capacity_limited

Source: `9ebc6da8a14b095d64ffa2f73398838cf487a6d8`. Recorded waves: 1/1; full 50: 0.

| Wave | Requested | Running | Refused | Context PSS MiB | Delta MiB | Outcome ms | Outcome / refusal |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| 1 | 50 | 31 | 19 | 150.37 | 69.53 | 128992.20 | 19× no_room (HTTP 503): No node can complete this request |


