# Process tool bundle inputs

This is K23's staged build input, not an enabled process capability. Native process tools
remain refused while confinement, relocation, lazy acquisition and hosted acceptance are built.
The existing container templates are unchanged.

[conda.lock.json](conda.lock.json) fixes every conda-forge archive by URL and SHA-256, including
transitive libraries, fonts and the external build tools. It was resolved for glibc 2.28;
the sysroot supplies the matching private loader and libraries. No package may be executed
before its archive is verified. micromamba 2.10.0's verified build-only input is recorded in
[its review](../../docs/checks/measurements/k23/micromamba-builder.json).

CPython 3.12.14 and Node 24.22.1 reuse K17's upstream runtime reviews. conda-pack 0.9.2
(BSD-3-Clause) and patchelf 0.19.2 (GPL-3.0-or-later) are separate build programs. Runtime
libraries retain their packaged licences, including LGPL, MPL and GCC's runtime exception;
none is linked into pryx. Use FreeType's FTL option and libev's BSD option. Retain the complete
notices and corresponding sources for redistributed copyleft components before publication.
The inventory records upstream declarations separately from full packaged notice hashes;
declarations alone are not a claim that every component has a single licence.

The source-reviewed corrections under `notices/` fill packaging gaps: PCRE2 10.47's
BSD-3-Clause with its binary-package exception, its pinned SLJIT's BSD-2-Clause, and Noto
Emoji 2.034's font-specific OFL-1.1. Its root Apache notice covers other files, and does not
replace the font notice. Their source archive digests are recorded in the decisions.

[artifacts.json](artifacts.json) pins only Chrome Headless Shell 158.0.8127.18, revision 1254.
Its complete 2,342,403-byte notice has exactly the hash reviewed in K18. The full Chromium
distribution and Widevine are excluded. Python wheels and JavaScript dependencies will use
the existing hash-locked interpreter templates. Nothing is downloaded on ordinary sandbox
creation merely because these build inputs exist.
