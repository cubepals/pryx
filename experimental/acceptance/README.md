# Default installation proof

The explicitly requested Any host proof runs the existing x64 and arm64 one-command jobs using the stock release assets. Their access logs must show that the install succeeded without requesting any experimental payload. The same jobs compare a second build of the pinned base made with their own preview key and staged runtime descriptors. Each bootstrap and host archive member must match by name, type, mode, owner and bytes. The comparison allows the native stock CLI executable to differ and compares its manifest entry and permissions. The installer is a separate authorized release change. Outer compression timestamps and preview signatures are not reproducibility evidence.

The paired bootstrap invokes each commit's own installer and CLI with PRYX_NO_UP in two fresh homes and compares the ordered GET requests. Full service startup, tier use and purge are exercised by the ordinary Any host acceptance on the candidate.

## Parts (`experimental/acceptance/`)

- `compare.sh` builds the reference and invokes both comparisons.
- `default_install.py` audits the real install log and the native bundle member manifests.
- `paired_install.py` captures paired default bootstrap requests.
- `README.md` describes this proof.
