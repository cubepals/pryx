# Experimental lab

The lab measures low-level implementations against the current pryx path and maintained libraries. A default install does not fetch a lab artifact. Each experiment uses a Cargo feature or standalone entry point and stays off until named. Results identify the exact commit, CPU, kernel, baseline and workload.

A change can leave the lab only after the end-to-end benchmark harness shows a gain on both x86-64 and aarch64, its scalar fallback has passed the differential tests, and the owner explicitly chooses to graduate it. A faster loop alone does not qualify.

## Parts (`experimental/`)

- [acceptance](acceptance/README.md) compares the default native installs with the pinned base.
- [install](install/README.md) packages the separate signed archive and tests feature selection.
- [proof](proof/README.md) provides the reference predicate, machine-code test crate, fuzz target and static reports.
- `Cargo.toml` defines a separate virtual workspace. Cargo excludes the lab tree when it packages the stock daemon or agent. Each lab crate retains its own manifest.
- `README.md` describes selection and graduation.

Run the scaffold proof through the existing Any host workflow with the `lab_scaffold` input on its experiment branch. The proof uses the same native installers and standard runners as a normal Any host dispatch. The lab adds no workflow that runs on push.
