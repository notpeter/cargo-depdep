# cargo-depdep

`cargo-depdep` compares the dependency lockfile in your working tree with the
lockfile at another Git revision. It supports Rust and npm projects, groups
every resolved version by package name, and prints the changes as a Markdown
table.

## Install

```console
cargo install --git https://github.com/notpeter/cargo-depdep
```

## Usage

### Help

```shell
Compare dependency lockfiles with another Git revision.

Usage: cargo depdep [OPTIONS] [ECOSYSTEM]

Arguments:
  [ECOSYSTEM]  Package ecosystem: rust or npm [default: auto-detect]

Options:
  --rev <REV>  Git rev to compare against [default: origin default branch, then local main/master]
  --all  Include transitive dependency changes
  --pretty  Align the columns for a nicely formatted ASCII table
  -h, --help    Print help
```

### Compact output

The default revision is the remote-tracking branch pointed to by `origin/HEAD`,
then `origin/main` or `origin/master`, then local `main` or `master`.
Remote-tracking branches reflect the last fetch; run `git fetch origin` to update
them without pulling. Use `--rev <REV>` to choose a revision explicitly.

`cargo depdep` detects `Cargo.toml` and `package.json` from the current directory
up to the repository root. If both are found, it compares both ecosystems in
separate Rust and npm sections. Each detected ecosystem requires its lockfile.
Pass `rust` or `npm` to compare only that ecosystem.

For a Rust project:

```console
# cargo depdep
| crate | old | new |
| --- | --- | --- |
| serde | 1.0.217 | 1.0.219 |
| syn | 1.0.109, 2.0.90 | 2.0.100 |
```

For an npm project, detection works automatically; you can also pass `npm`:

```console
# cargo depdep npm
| package | old | new |
| --- | --- | --- |
| eslint | 9.31.0 | 9.33.0 |
| typescript | 5.8.3 | 5.9.2 |
```

By default, the table only includes direct dependencies declared in
`Cargo.toml` or `package.json`. If other resolved packages changed, the command
prints a hint to stderr:

```console
4 additional transitive dependency changes not shown; use --all to show them
```

Pass `--all` to include those packages in the table.

### Pretty output

Pass `--pretty` get a nicely formatted space-padded table:

Example output:

```markdown
| crate |             old |     new |
| :---- | --------------: | ------: |
| serde |         1.0.217 | 1.0.219 |
| syn   | 1.0.109, 2.0.90 | 2.0.100 |
```

## Implementation

The command searches from the current directory toward the repository root. A
Rust project is identified by `Cargo.toml` and `Cargo.lock`; an npm project uses
`package.json` and `package-lock.json`. The `old` column comes from the selected
revision and the `new` column comes from the working tree. Added and removed
packages have an empty cell. Direct dependencies are taken from both versions
of the manifest so that additions and removals are included. At a Rust workspace
root, direct dependencies are collected from its member manifests on each side
of the comparison, including workspace-inherited dependencies. Member patterns
and exclusions are respected; unused workspace dependency declarations are not
counted as direct dependencies. Resolved npm
versions come from `package-lock.json`.

No deps!

## License

MIT
