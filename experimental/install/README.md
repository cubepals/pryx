# Installing the lab

A stock install makes no lab request. The installer accepts `sh -s -- --experimental` when reading the release installer from a pipe. The same switch works on `pryx up`. Both accept `PRYX_EXPERIMENTAL=1` in the environment of the installer or CLI. Installation activates no experiment unless the invocation also names it.

For example, `pryx up --experimental --feature self-check` installs the lab and selects its differential self-check. `pryx experimental run self-check` runs it. `pryx doctor`, `pryx doctor --json` and `pryx capabilities` show the current user's selection and reason. Pass `--experimental` to doctor on a clean home to see why the lab is off. Running `pryx up --experimental` again clears the selection. Preflight and check modes never install the lab.

The stock CLI downloads `pryx-lab-linux-x64.pyz` or `pryx-lab-linux-arm64.pyz` and its adjacent `.sig` from the versioned release location. The Ed25519 signature binds the archive digest, architecture, version and lab-specific purpose to the CLI's compiled release keys. A private copy is verified before every invocation. The signed archive requires a local Python 3.8+ interpreter. Only a named run extracts and executes its native binary. The installed generation and selection are local to the calling user under `~/.pryx-experimental`. Activation replaces one directory link after verification and successful stock setup. A failed validation keeps the previous selection.

The release owner must make the signed assets available before a production opt-in can succeed. The experiment workflow can build preview artifacts. The default release asset build and its automatic workflows do not publish a lab archive. The handoff changes the stock CLI and installer bytes. Existing one-command acceptance and comparisons must treat those two changes explicitly.

## Packaging a release artifact

Use the native binary built for the target machine. The version argument must match the stock release. The example's private-key path refers to the existing protected signing environment.

```sh
python3 experimental/install/package.py "$LAB_BINARY" "$ARTIFACT" --arch "$ARCH"
python3 -B experimental/install/sign.py "$ARTIFACT" "$ARTIFACT.sig" --version "$VERSION" --arch "$ARCH" --key "$RELEASE_KEY_PATH"
```

Run `python3 -B experimental/install/acceptance.py` as an ordinary user after the repository's frozen Bun install. It builds the real CLI with a disposable fixture key and checks both stock command entry points against a local release server. Set `PRYX_LAB_TEST_BINARY` to the native Rust lab binary to package it and require its self-check to pass. With that variable unset, the local smoke uses an exit-only executable.

Parts (`install/`):

- `README.md` describes opt-in, verification and publication.
- `acceptance.py` exercises the stock installer and CLI, explicit selection, refusals and rollback.
- `lab.py` activates an authenticated generation, reports selection and runs the named native feature.
- `lab_release.py` constructs the architecture- and version-bound signing statement.
- `package.py` checks the native ELF architecture and packs the executable with its lab frontend.
- `sign.py` signs a prepared archive with the release signer.
