# cargo-depdep

`cargo-depdep` compares the dependency lockfile in your working tree with the
lockfile at another Git revision. It supports Rust, npm, and Ruby projects, groups
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
  [ECOSYSTEM]  Package ecosystem: rust, npm, or ruby [default: auto-detect]

Options:
  --rev <REV>  Git rev to compare against [default: origin default branch, then local main/master]
  --all  Include transitive dependency changes
  --security  List fixed security advisories before the table
  --fancy  Link advisory IDs in Markdown (requires --security)
  --pretty  Align the columns for a nicely formatted ASCII table
  -h, --help    Print help
```

### Compact output

The default revision is the remote-tracking branch pointed to by `origin/HEAD`,
then `origin/main` or `origin/master`, then local `main` or `master`.
Remote-tracking branches reflect the last fetch; run `git fetch origin` to update
them without pulling. Use `--rev <REV>` to choose a revision explicitly.

`cargo depdep` finds lockfiles recursively across the Git repository, including
new, untracked projects and excluding ignored files. It compares each project
with the selected revision and skips unchanged dependencies. Under
`## Package Changes`, results are grouped under their lockfile paths, such as
`xtask/package-lock.json` and `rfd-processor/package-lock.json`.

Pass `rust`, `npm`, or `ruby` to filter by ecosystem. Ruby also accepts `gem`
and `gems` as aliases. Discovery covers the whole repository even when run from
a subdirectory.

For a Rust project:

```console
# cargo depdep
## Package Changes

### Cargo.lock

| crate | old | new |
| --- | --- | --- |
| serde | 1.0.217 | 1.0.219 |
| syn | 1.0.109, 2.0.90 | 2.0.100 |
```

For an npm project, detection works automatically; you can also pass `npm`:

```console
# cargo depdep npm
## Package Changes

### package-lock.json

| package | old | new |
| --- | --- | --- |
| eslint | 9.31.0 | 9.33.0 |
| typescript | 5.8.3 | 5.9.2 |
```

For a Ruby project using Bundler:

```console
# cargo depdep ruby
## Package Changes

### Gemfile.lock

| gem | old | new |
| --- | --- | --- |
| asciidoctor | 2.0.23 | 2.0.26 |
| rouge | 4.6.1 | 5.0.0 |
```

By default, the table only includes direct dependencies declared in
`Cargo.toml` or `package.json`, or recorded in `Gemfile.lock` under
`DEPENDENCIES`. If other resolved packages changed, the command
prints a hint to stderr:

```console
4 additional transitive dependency changes not shown; use --all to show them
```

Pass `--all` to include those packages in the table.

### Security fixes

Pass `--security` to list security advisories fixed between the selected
revision and the working tree. Fixes from all projects are grouped and deduplicated
under `## Security updates`, with one bullet per advisory and its title on the
following indented line, followed by the package changes:

```console
# cargo depdep npm --security
## Security updates

- Fixes GHSA-27r3-jxmh-p828 (body-parser)
  body-parser: denial of service when URL encoding is used
- Fixes GHSA-6pq8-jhm2-2vcw (qs)
  qs: prototype pollution

## Package Changes

### package-lock.json

| package | old | new |
| --- | --- | --- |
| body-parser | 2.2.0 | 2.2.1 |
```

Add `--fancy` to link GHSA, CVE, and RUSTSEC IDs to their advisory pages:

```markdown
- Fixes [GHSA-27r3-jxmh-p828](https://github.com/advisories/GHSA-27r3-jxmh-p828) (body-parser)
  body-parser: denial of service when URL encoding is used
```

For npm, requires npm 7 or newer and network access. Uses `npm audit` without
changing your project, including transitive and development dependencies. The first run
can be slow; cached metadata speeds up later runs while advisory reports stay
fresh.

For Rust, use `cargo depdep rust --security`. Requires `cargo-audit`
(`cargo install cargo-audit`) and network access. Reports fixed advisories
using a cached advisory database, including transitive dependencies. GHSA aliases,
when available, appear before the RUSTSEC ID; `--fancy` links each ID:

```console
- Fixes GHSA-vp88-798v-h67f RUSTSEC-2026-0347 (rustls)
  Buffered application data sent to outer server after ECH rejection
```

Ruby security reporting is not implemented.

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

The command uses Git to discover lockfiles throughout the repository. A
Rust project is identified by `Cargo.toml` and `Cargo.lock`; an npm project uses
`package.json` and `package-lock.json`; a Ruby project uses `Gemfile` and
`Gemfile.lock`. The `old` column comes from the selected
revision and the `new` column comes from the working tree. Added and removed
packages have an empty cell. Direct dependencies are taken from both revisions
so that additions and removals are included. At a Rust workspace
root, direct dependencies are collected from its member manifests on each side
of the comparison, including workspace-inherited dependencies. Member patterns
and exclusions are respected; unused workspace dependency declarations are not
counted as direct dependencies. Resolved npm
versions come from `package-lock.json`. Ruby versions and direct dependencies
come from `Gemfile.lock`, without executing the Gemfile or requiring Ruby or
Bundler to be installed. Gems from registry, Git, and path sources are included,
as are platform-specific versions. Changes to source URLs or Git revisions alone
are not reported when gem versions stay the same.

No deps!

## License

MIT
