# Templates from code

`pryx.template({ name, from, copy, run, env, workdir, ready, start, sparePorts })` prepares a
sandbox using the existing files and process APIs, snapshots it, and registers an immutable
version. `copy` stays local; only the build plan and completed snapshot reach control. Failed
builds clean up their sandbox. `template.rebuild()` creates the next version. The Python sync
and async clients accept the same plan fields.

`pryx template build -f pryx.template.ts` runs that builder. `pryx template create` supplies
built-in plans for `python-interpreter`, `node-interpreter`, `desktop`, `browser` and `tools`.
`--image` selects an operator-built image. Use `pryx.sandbox({ template: name })` after warming.
See [interpreters](interpreter.md) and [native tools](native-tools.md) for their dependencies,
capabilities, host prerequisites and security limits. Creation from a ready template clones its
snapshot; it never repeats package installation on the caller's first command.

The implementation is [the SDK builder](../sdk/typescript/src/template.ts),
[version registration](../control/src/code-templates.ts), and
[the built-in CLI plans](../cli/src/builtin-template.ts), covered by SDK, public API and deployment CI.
