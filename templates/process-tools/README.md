# Process tool bundle

K23's ordinary-user browser and Python/Node kernels run without Docker, proved by the
CI evidence in [the progress record](../../docs/any-host-progress.md). Its inputs are upstream distributions that
relocate by design: python-build-standalone CPython 3.12.16 (20261110), Node's official 24.22.1
tarball and Chrome Headless Shell 158.0.8127.18. [artifacts.json](artifacts.json) pins every
archive by SHA-256. The Python install-only interpreter matches the separately pinned full
build exactly; retain that build's complete runtime notices and linkage metadata.

Launchers set the library path to the bundle's own libraries and execute the unchanged
upstream binaries through **host glibc**. No binary rewriting, private loader or private glibc
belongs in this bundle. The complete locked dependency set requires host glibc **2.34**
(debugpy's attach helper sets the measured floor). The host probe reports its actual version; tools whose minimum
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

`sources.py` builds the separate corresponding-source release asset from the digest-pinned
`sources.lock.json`, retaining upstream archives and the exact binary packages' complete
recipes and patches. Runtime notices record that asset's descriptor; asking for a tool never
downloads its sources. Both assets must accompany production publication. Source collection
never runs a package recipe or modifies an upstream binary.

The release build writes its generated descriptor under `deploy/process-tools/` before image
packaging. Both the full signed host bundle and the bootstrap retain it beside the actual
agent executable. The large runtime and source assets remain outside those installations.
A source-only image build without generated metadata keeps native tools unavailable with
the existing missing-artifact reason.

Parts (`process-tools/`):

- `build.py`: verified assembly, complete notices, addon selection and an ELF symbol audit.
- `archive_inputs.py`: immutable archive retrieval and payload copying without package hooks.
- `package.py`: unchanged payload packaging and pinned archive/inventory expansion metadata.
- `package_test.py`: real archive determinism, bounds and inventory-tampering regressions.
- `sources.py`: separate corresponding-source distribution and runtime notice descriptor.
- `sources.lock.json`: immutable source archives for copyleft and Sleepycat components.
- `smoke.py`: real stateful Python/Node kernels and charts after physical relocation.
- `browser/`: maintained debugging-pipe and Unix-proxy adapters, lock and browser acceptance.
- `launchers/`: small runtime launchers using the bundle's libraries and host glibc.
- `artifacts.json`: immutable upstream runtime and matching notice archives.
- `compatibility.json`: one architecture and host-glibc prerequisite for build and host probe.
- `desktop.py`: separately staged X11 assembly and own-library/host-glibc audit, not published yet.
- `desktop-fixture.py`: unchanged X11 inputs for the initial argument design test.
- `desktop.lock.json`: digest-pinned staged Debian programs, libraries and full copyright hashes.
- `desktop_test.py`: real staged ELF missing-library and private-glibc rejection contracts.
- `desktop_sources.py`: complete DSC-verified original sources and Debian changes, without execution.
- `desktop-sources.lock.json`: immutable corresponding sources for every staged binary source/version.
- `desktop_sources_test.py`: real DSC rejection when a required original archive is omitted.
- `desktop_full.py`: separately staged headful Chromium, VNC/capture and OpenSSH closure; no runtime publication yet.
- `desktop-full.json`: measured full-component host floor and unchanged program paths.
- `desktop-full.lock.json`: exact selected ELF files, runtime data packages and notice digests.
- `desktop-full-sources.lock.json`: complete DSC/original/Debian source inputs for that component.
- `desktop_full_test.py`: actual full-component missing-library and private-glibc rejection.
- `desktop_web.py`: verified noVNC/websockify staging and complete source retention.
- `desktop-web.lock.json`: pinned web programs and corresponding sources, reusing K18 reviews.
- `desktop/`: fixed external desktop supervision, Unix forwarding and token expiry adapters.
- `libraries.lock.json`: immutable library/font archives and their notice hashes.
- `helpers.lock.json`: separately executed namespace helpers and their complete notices.
- `notices/`: source-reviewed corrections to incomplete library/font package notices.
