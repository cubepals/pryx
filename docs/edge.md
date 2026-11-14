# Silent-client measurements

Run on Linux with a TCP listener that accepts but never writes, using clients with their default timeouts. `n=3` per client/protocol; each observation is capped at 15 seconds. Censored rows show lower bounds, not estimated timeouts. p90 uses the nearest-rank sample maximum. These measurements characterize these versions on this host, not every client version.

| Client | Wait | n | p50 seconds | p90 seconds | Censored |
| --- | --- | --- | --- | --- | --- |
| Chrome | HTTP | 3 | ≥ 15.034 | ≥ 15.048 | 3/3 |
| Chrome | TLS | 3 | ≥ 15.040 | ≥ 15.049 | 3/3 |
| Go net/http | HTTP | 3 | ≥ 15.005 | ≥ 15.014 | 3/3 |
| Go net/http | TLS | 3 | 10.064 | 10.076 | 0/3 |
| Node fetch | HTTP | 3 | ≥ 15.014 | ≥ 15.295 | 3/3 |
| Node fetch | TLS | 3 | 10.724 | 10.779 | 0/3 |
| OpenSSH | banner | 3 | ≥ 15.023 | ≥ 15.030 | 3/3 |
| Python requests | HTTP | 3 | ≥ 15.036 | ≥ 15.039 | 3/3 |
| Python requests | TLS | 3 | ≥ 15.009 | ≥ 15.011 | 3/3 |
| curl | HTTP | 3 | ≥ 15.049 | ≥ 15.162 | 3/3 |
| curl | TLS | 3 | ≥ 15.048 | ≥ 15.049 | 3/3 |
| Safari | TLS and HTTP | 0 | unavailable | unavailable | Safari cannot run on this Linux host |

The default hold is **5 seconds**, below the smallest uncensored bound measured: Go net/http TLS (about 10 seconds). Node fetch TLS also timed out near 11 seconds. The other clients stayed connected through the cap. Safari is unmeasured, so no Safari bound or universal guarantee is claimed. A longer configured hold may help slow boots but cannot extend a client timeout.

Commands and raw records:

```sh
python3 -m venv ~/.local/pryx-edge-measure
~/.local/pryx-edge-measure/bin/pip install requests==2.32.5
export PATH="$HOME/.local/pryx-edge-measure/bin:$HOME/.local/go/bin:$PATH"
python3 control/tools/measure-edge.py
```

The script builds a standard Go HTTP client, runs curl without a max-time, Node fetch without an AbortSignal, Python requests without a timeout, Go http.Get, Chromium headless with dump-dom, and ssh with no ConnectTimeout. Chromium uses no-sandbox only for this local test. Each capped process group is killed at the observation boundary. Go was installed from its published archive with its published SHA256 checked.

Versions:

- curl 8.14.1 (x86_64-pc-linux-gnu) libcurl/8.14.1 OpenSSL/3.5.7 zlib/1.3.1 brotli/1.1.0 zstd/1.5.7 libidn2/2.3.8 libpsl/0.21.2 libssh2/1.11.1 nghttp2/1.64.0 nghttp3/1.8.0 librtmp/2.3 OpenLDAP/2.6.10
- v24.22.1
- 2.32.5
- go version go1.27.3 linux/amd64
- Chromium 155.0.8059.79 built on Debian GNU/Linux 13 (trixie)
- OpenSSH_10.0p2 Debian-7+deb13u4, OpenSSL 3.5.7 9 Jun 2026

Final measurement summaries: [edge-clients.json](checks/edge-clients.json). The first attempt found an unrelated `go` program; a later diagnostic found requests missing. The diagnostic [initial samples](checks/edge-clients-initial.json) retain that failure; its Python exits are excluded from the table. The successful rerun used requests 2.32.5.

The edge keeps TLS ClientHello and HTTP request bytes, sends no hold messages, and splices them once control wakes the sandbox. Snapshots restore files, so a rest wake can take longer than a memory resume. Choose the configured hold using the clients and images actually deployed. A failed hold closes; the caller may reconnect after startup finishes.

Control uses database-generated route versions and records each edge’s last fully held version. A running sandbox is shown as starting until every counted edge holds the first version naming it running. The adapted Cubepals liveness rules exclude quiet/behind edges and bound that display wait to 15 seconds; they do not fence a node or authorize failover. Route epochs fence edge connections independently.
