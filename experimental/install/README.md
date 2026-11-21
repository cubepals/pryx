# Installing the lab

A stock install makes no lab request. The installer accepts `sh -s -- --experimental` when reading the release installer from a pipe. The same switch works on `pryx up`. Both accept `PRYX_EXPERIMENTAL=1` in the environment of the installer or CLI. Installation activates no experiment unless the invocation also names it.

For example, `pryx up --experimental --feature self-check` installs the lab and selects its differential self-check. `pryx experimental run self-check` runs it. `pryx doctor`, `pryx doctor --json` and `pryx capabilities` show the current user's selection and reason. Pass `--experimental` to doctor on a clean home to see why the lab is off. Running `pryx up --experimental` again clears the selection. Preflight and check modes validate the argument grammar offline. If a name is supplied, they report that its availability is unverified until installation authenticates the archive. The CLI accepts one lowercase ASCII name, starting with a letter, using letters, digits or hyphens, up to 64 characters. The archive decides whether that name exists.

The installer forwards lab arguments to the CLI on explicit request. Its normal host preset and domain still come from the stock installer. The trusted CLI must perform the bounded download, verify the archive against its compiled release keys, and recheck a protected private copy on each handoff. This verification runs before any code from the archive can execute. Feature names, selection storage, capability reasons and native implementations belong to the signed archive.

The stock CLI downloads `pryx-lab-linux-x64.pyz` or `pryx-lab-linux-arm64.pyz` and its adjacent `.sig` from the versioned release location. The Ed25519 signature binds the archive digest, architecture, version and lab-specific purpose to the CLI's compiled release keys. A private copy is verified before every invocation. The signed archive requires a local Python 3.8+ interpreter. Only a named run extracts and executes its native binary. The installed generation and selection are local to the calling user under `~/.pryx-experimental`. Activation replaces one directory link after verification and successful stock setup. A generation digest binds the authenticated archive and its detached signature to the local selector. Doctor and run check the recorded selector against that identity. Installers serialize activation under the private directory lock and reuse intact generations, including one prepared before an interrupted pointer switch. Repairing a damaged generation stages a fresh copy under a separate name and atomically switches the active link. A reader using the previous directory can finish independently. To repair only the lab, run `pryx experimental install --experimental` with the desired `--feature`. A failed validation keeps the previous selection.

For an unreleased version, the owner can select “Include separately signed experimental archives” when dispatching the existing Release workflow on main. The dispatch can be queued while CI runs on the new main commit. It checks the main push CI result for that exact commit, the existing new-version gate and the main-ref conditions on both signing jobs before publication. The signer reads the existing key from the release environment. The normal release will still publish without lab files if it takes the version first. Published releases are immutable; once the version has a tag or release, a later dispatch leaves it alone.

The arm64 and x64 release workers build native static musl executables and run their self-checks. Each worker authenticates its archive with the pinned release public key after signing. The four separate files enter the existing provenance attestation and initial release draft after the standard checksum manifest has been finalized. The stock installer and ordinary host bundle do not fetch the lab files. A requested lab install on a release without those optional assets reports their absence. The handoff changes the stock CLI and installer bytes; the existing one-command acceptance compares those changes explicitly.

## Packaging a release artifact

The release workflow calls `build-release.sh` on each native machine with its Rust musl target installed. The build uses the bundled Rust linker and inspects the resulting ELF for an interpreter or needed dynamic libraries before exercising it. To package an already-built native executable, use the commands below. The version must match the stock release and the private-key path refers to the release signer's key.

```sh
python3 experimental/install/package.py "$LAB_BINARY" "$ARTIFACT" --arch "$ARCH"
python3 -B experimental/install/sign.py "$ARTIFACT" "$ARTIFACT.sig" --version "$VERSION" --arch "$ARCH" --key "$RELEASE_KEY_PATH" --public-key "$VERIFIED_RELEASE_PUBLIC_KEY"
```

Run `python3 -B experimental/install/acceptance.py` as an ordinary user after the repository's frozen Bun install. It builds the real CLI with a disposable fixture key and checks both stock command entry points against a local release server. Set `PRYX_LAB_TEST_BINARY` to the native Rust lab binary to package it and require its self-check to pass. With that variable unset, the local smoke uses an exit-only executable.

Parts (`install/`):

- `README.md` describes opt-in, verification and publication.
- `build-release.sh` builds and verifies a native static musl executable and packs its release archive.
- `acceptance.py` exercises the stock installer and CLI, explicit selection, refusals and rollback.
- `lab.py` activates an authenticated generation, reports selection and runs the named native feature.
- `lab_release.py` constructs the architecture- and version-bound signing statement.
- `package.py` checks the native ELF architecture and packs the executable with its lab frontend.
- `sign.py` signs a prepared archive with the release signer.
