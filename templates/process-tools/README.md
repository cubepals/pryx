# Process tool bundle

K23's main deliverable remains active until an ordinary user's install runs its browser and
Python/Node kernels without Docker, proved in CI. Its inputs are upstream distributions that
relocate by design: python-build-standalone CPython 3.12.16 (20261110), Node's official 24.22.1
tarball and Chrome Headless Shell 158.0.8127.18. [artifacts.json](artifacts.json) pins every
archive by SHA-256. The Python install-only interpreter matches the separately pinned full
build exactly; retain that build's complete runtime notices and linkage metadata.

Launchers set the library path to the bundle's own libraries and execute the unchanged
upstream binaries through **host glibc**. No binary rewriting, private loader or private glibc
belongs in this bundle. The host probe reports its actual glibc version; tools whose minimum
cannot be met stay off with that reason. The completed bundle is verified by digest and fetched
only when a sandbox first asks for a native tool. Its cache and private runtime state stay
outside persistent sandbox data.

[libraries.lock.json](libraries.lock.json) pins only library/font inputs from conda-forge.
Their complete notices, source directions, FreeType FTL choice and libev BSD choice remain
required. Python wheels and Node kernel packages use the existing hash-locked interpreter
inputs. [The standalone-Python review](../../docs/checks/measurements/k23/python-standalone-review.json)
records PSF/CNRI and dependency licences, including the optional Berkeley DB extension's
Sleepycat source obligations. These are separate external programs and libraries, never linked
into pryx. The Node runtime notice exactly matches the previously reviewed version, and the
headless shell's complete notice matches K18. Full Chromium and Widevine remain excluded.

The failed conda-prefix/private-loader experiments are retained as
[historical evidence](../../docs/checks/measurements/k23/abandoned-conda-prefix.lock.json).
Their proposed parking and binary-rewriting retry are superseded by the user's correction.
They are not build inputs for the active bundle. The main deliverable is never parked.

Parts (`process-tools/`):

- `launchers/`: small runtime launchers using the bundle's libraries and host glibc.
- `artifacts.json`: immutable upstream runtime and matching notice archives.
- `libraries.lock.json`: immutable library/font archives and their notice hashes.
- `notices/`: source-reviewed corrections to incomplete library/font package notices.
