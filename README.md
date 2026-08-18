# cargo-depdep

`cargo-depdep` compares the `Cargo.lock` in your working tree with the lockfile
at another Git revision. It groups every version by crate name and prints the
changed crates as a Markdown table.

## Install

```console
cargo install --git https://github.com/notpeter/cargo-depdep
```

## Usage

### Help

```shell
Compare the working tree's Cargo.lock with another Git revision.

Usage: cargo depdep [OPTIONS]

Options:
  --rev <REV>  Git rev to compare against [default: main or repo default branch]
  --pretty  Align the columns for a nicely formatted ASCII table
  -h, --help    Print help
```

### Compact output

```console
# cargo depdep
| crate | old | new |
| --- | --- | --- |
| serde | 1.0.217 | 1.0.219 |
| syn | 1.0.109, 2.0.90 | 2.0.100 |
```

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

The command searches from the current directory toward the repository root for
the nearest `Cargo.lock`. The `old` column comes from the selected revision and
the `new` column comes from the working tree. Added and removed crates have an
empty cell.

No deps!

## License

MIT
