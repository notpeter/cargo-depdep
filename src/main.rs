use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{self, Command};

const DEFAULT_REVISION: &str = "main";
const LOCKFILE_NAME: &str = "Cargo.lock";

fn main() {
    if let Err(error) = run(env::args_os().skip(1)) {
        eprintln!("error: {error}");
        process::exit(1);
    }
}

fn run(args: impl IntoIterator<Item = OsString>) -> Result<(), Error> {
    let options = parse_args(args)?;
    let current_dir = env::current_dir().map_err(|source| Error::Io {
        context: "could not determine the current directory".into(),
        source,
    })?;
    let repository_root = git_repository_root(&current_dir)?;
    let revision = match options.revision {
        Some(revision) => revision,
        None => default_revision(&repository_root),
    };
    let lockfile = find_lockfile(&current_dir, &repository_root)?;
    let lockfile_path = lockfile
        .strip_prefix(&repository_root)
        .expect("the lockfile search stays inside the repository");

    let old_contents = lockfile_at_revision(&repository_root, &revision, lockfile_path)?;
    let new_contents = fs::read_to_string(&lockfile).map_err(|source| Error::Io {
        context: format!("could not read {}", lockfile.display()),
        source,
    })?;

    let old_packages = parse_lockfile(&old_contents).map_err(|message| Error::Lockfile {
        label: format!("{revision}:{}", lockfile_path.display()),
        message,
    })?;
    let new_packages = parse_lockfile(&new_contents).map_err(|message| Error::Lockfile {
        label: lockfile.display().to_string(),
        message,
    })?;

    print_diff(&old_packages, &new_packages, options.pretty);
    Ok(())
}

struct Options {
    revision: Option<String>,
    pretty: bool,
}

fn parse_args(args: impl IntoIterator<Item = OsString>) -> Result<Options, Error> {
    let mut args = args.into_iter().peekable();

    // Cargo invokes an external subcommand as `cargo-depdep depdep ...`.
    // Keep direct `cargo-depdep ...` invocation working too.
    if args.peek().and_then(|arg| arg.to_str()) == Some("depdep") {
        args.next();
    }

    let mut revision = None;
    let mut pretty = false;

    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("-h" | "--help") => {
                println!(
                    "Compare the working tree's Cargo.lock with another Git revision.\n\n\
                     Usage: cargo depdep [OPTIONS]\n\n\
                     Options:\n  \
                       --rev <REV>  Git rev to compare against [default: main or repo default branch]\n  \
                       --pretty  Align the columns for a nicely formatted ASCII table\n  \
                   -h, --help    Print help"
                );
                process::exit(0);
            }
            Some("--pretty") => pretty = true,
            Some("--rev" | "--branch") => {
                if revision.is_some() {
                    return Err(Error::Usage("--rev may only be used once".into()));
                }
                let value = args
                    .next()
                    .ok_or_else(|| Error::Usage("--rev requires a value".into()))?;
                revision = Some(
                    value
                        .into_string()
                        .map_err(|_| Error::Usage("the revision must be valid UTF-8".into()))?,
                );
            }
            _ => {
                return Err(Error::Usage(format!(
                    "unexpected argument {:?}",
                    arg.to_string_lossy()
                )));
            }
        }
    }

    Ok(Options { revision, pretty })
}

fn git_repository_root(current_dir: &Path) -> Result<PathBuf, Error> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(current_dir)
        .output()
        .map_err(|source| Error::Io {
            context: "could not run git".into(),
            source,
        })?;

    if !output.status.success() {
        return Err(Error::Git(command_error(&output.stderr)));
    }

    let root = String::from_utf8(output.stdout)
        .map_err(|_| Error::Git("git returned a non-UTF-8 repository path".into()))?;
    Ok(PathBuf::from(root.trim_end()))
}

fn default_revision(repository_root: &Path) -> String {
    // Prefer the branch the remote's HEAD points at, if it is configured.
    if let Some(reference) = git_line(
        repository_root,
        &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
    ) {
        if let Some(branch) = reference.rsplit('/').next().filter(|name| !name.is_empty()) {
            return branch.to_string();
        }
    }

    // Otherwise fall back to whichever conventional branch exists locally.
    for candidate in ["main", "master"] {
        if git_line(
            repository_root,
            &["rev-parse", "--verify", "--quiet", candidate],
        )
        .is_some()
        {
            return candidate.to_string();
        }
    }

    DEFAULT_REVISION.into()
}

fn git_line(repository_root: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repository_root)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let line = String::from_utf8(output.stdout).ok()?;
    let line = line.trim();
    (!line.is_empty()).then(|| line.to_string())
}

fn find_lockfile(current_dir: &Path, repository_root: &Path) -> Result<PathBuf, Error> {
    let mut directory = current_dir;

    loop {
        let candidate = directory.join(LOCKFILE_NAME);
        if candidate.is_file() {
            return Ok(candidate);
        }
        if directory == repository_root {
            break;
        }
        directory = directory.parent().ok_or_else(|| {
            Error::Usage(format!(
                "could not find {LOCKFILE_NAME} between {} and {}",
                current_dir.display(),
                repository_root.display()
            ))
        })?;
    }

    Err(Error::Usage(format!(
        "could not find {LOCKFILE_NAME} between {} and {}",
        current_dir.display(),
        repository_root.display()
    )))
}

fn lockfile_at_revision(
    repository_root: &Path,
    revision: &str,
    lockfile_path: &Path,
) -> Result<String, Error> {
    let object = format!("{revision}:{}", lockfile_path.to_string_lossy());
    let output = Command::new("git")
        .args(["show", "--no-ext-diff", &object])
        .current_dir(repository_root)
        .output()
        .map_err(|source| Error::Io {
            context: "could not run git".into(),
            source,
        })?;

    if !output.status.success() {
        return Err(Error::Git(format!(
            "could not read {object}: {}",
            command_error(&output.stderr)
        )));
    }

    String::from_utf8(output.stdout).map_err(|_| Error::Git(format!("{object} is not valid UTF-8")))
}

fn command_error(stderr: &[u8]) -> String {
    let message = String::from_utf8_lossy(stderr);
    let message = message.trim();
    if message.is_empty() {
        "git exited unsuccessfully".into()
    } else {
        message.into()
    }
}

type Packages = BTreeMap<String, BTreeSet<String>>;

fn parse_lockfile(contents: &str) -> Result<Packages, String> {
    let mut packages = Packages::new();
    let mut in_package = false;
    let mut name = None;
    let mut version = None;

    for (index, line) in contents.lines().enumerate() {
        let line_number = index + 1;
        let trimmed = line.trim();

        if trimmed.starts_with("[[") && trimmed.ends_with("]]") {
            if in_package {
                add_package(&mut packages, name.take(), version.take(), line_number)?;
            }
            in_package = trimmed == "[[package]]";
            continue;
        }

        if !in_package {
            continue;
        }

        if let Some(value) = assignment(trimmed, "name") {
            name = Some(parse_toml_string(value, line_number)?);
        } else if let Some(value) = assignment(trimmed, "version") {
            version = Some(parse_toml_string(value, line_number)?);
        }
    }

    if in_package {
        add_package(&mut packages, name, version, contents.lines().count() + 1)?;
    }

    Ok(packages)
}

fn assignment<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let (candidate, value) = line.split_once('=')?;
    (candidate.trim() == key).then(|| value.trim())
}

fn parse_toml_string(value: &str, line_number: usize) -> Result<String, String> {
    let Some(inner) = value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
    else {
        return Err(format!("line {line_number}: expected a quoted string"));
    };

    let mut parsed = String::new();
    let mut chars = inner.chars();
    while let Some(character) = chars.next() {
        if character != '\\' {
            parsed.push(character);
            continue;
        }

        let escaped = chars
            .next()
            .ok_or_else(|| format!("line {line_number}: incomplete string escape"))?;
        parsed.push(match escaped {
            'b' => '\u{0008}',
            't' => '\t',
            'n' => '\n',
            'f' => '\u{000c}',
            'r' => '\r',
            '"' => '"',
            '\\' => '\\',
            _ => return Err(format!("line {line_number}: unsupported string escape")),
        });
    }
    Ok(parsed)
}

fn add_package(
    packages: &mut Packages,
    name: Option<String>,
    version: Option<String>,
    line_number: usize,
) -> Result<(), String> {
    let name = name
        .ok_or_else(|| format!("package ending before line {line_number} does not have a name"))?;
    let version = version.ok_or_else(|| {
        format!("package {name:?} ending before line {line_number} does not have a version")
    })?;
    packages.entry(name).or_default().insert(version);
    Ok(())
}

fn print_diff(old: &Packages, new: &Packages, pretty: bool) {
    let mut rows = Vec::new();
    for name in old.keys().chain(new.keys()).collect::<BTreeSet<_>>() {
        if old.get(name) == new.get(name) {
            continue;
        }

        rows.push([
            escape_markdown(name),
            versions(old.get(name)),
            versions(new.get(name)),
        ]);
    }

    if rows.is_empty() {
        eprintln!("no changes");
        return;
    }

    if pretty {
        print_pretty(&rows);
    } else {
        print_plain(&rows);
    }
}

const HEADERS: [&str; 3] = ["crate", "old", "new"];

fn print_plain(rows: &[[String; 3]]) {
    println!("| {} | {} | {} |", HEADERS[0], HEADERS[1], HEADERS[2]);
    println!("| --- | --- | --- |");

    for row in rows {
        println!("| {} | {} | {} |", row[0], row[1], row[2]);
    }
}

fn print_pretty(rows: &[[String; 3]]) {
    let mut widths = HEADERS.map(|header| header.chars().count());
    for row in rows {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.chars().count());
        }
    }

    // The crate column stays left-aligned; the version columns are right-aligned.
    println!(
        "| {:<crate$} | {:>old$} | {:>new$} |",
        HEADERS[0],
        HEADERS[1],
        HEADERS[2],
        crate = widths[0],
        old = widths[1],
        new = widths[2],
    );
    println!(
        "| {} | {} | {} |",
        separator(widths[0], false),
        separator(widths[1], true),
        separator(widths[2], true),
    );

    for row in rows {
        println!(
            "| {:<crate$} | {:>old$} | {:>new$} |",
            row[0],
            row[1],
            row[2],
            crate = widths[0],
            old = widths[1],
            new = widths[2],
        );
    }
}

fn separator(width: usize, right_aligned: bool) -> String {
    let dashes = "-".repeat(width.saturating_sub(1));
    if right_aligned {
        format!("{dashes}:")
    } else {
        format!(":{dashes}")
    }
}

fn versions(versions: Option<&BTreeSet<String>>) -> String {
    versions
        .into_iter()
        .flatten()
        .map(|version| escape_markdown(version))
        .collect::<Vec<_>>()
        .join(", ")
}

fn escape_markdown(value: &str) -> String {
    value.replace('\\', "\\\\").replace('|', "\\|")
}

#[derive(Debug)]
enum Error {
    Usage(String),
    Git(String),
    Lockfile {
        label: String,
        message: String,
    },
    Io {
        context: String,
        source: std::io::Error,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usage(message) | Self::Git(message) => formatter.write_str(message),
            Self::Lockfile { label, message } => {
                write!(formatter, "could not parse {label}: {message}")
            }
            Self::Io { context, source } => write!(formatter, "{context}: {source}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_groups_package_versions() {
        let packages = parse_lockfile(
            r#"
version = 4

[[package]]
name = "same-crate"
version = "1.0.0"

[[package]]
name = "same-crate"
version = "2.0.0"

[[package]]
name = "another-crate"
version = "3.1.4"
"#,
        )
        .unwrap();

        assert_eq!(
            packages["same-crate"],
            BTreeSet::from(["1.0.0".into(), "2.0.0".into()])
        );
        assert_eq!(packages["another-crate"], BTreeSet::from(["3.1.4".into()]));
    }

    #[test]
    fn ignores_non_package_tables() {
        let packages = parse_lockfile(
            r#"
version = 4

[[metadata]]
name = "not-a-package"
version = "1.0.0"
"#,
        )
        .unwrap();

        assert!(packages.is_empty());
    }

    #[test]
    fn escapes_markdown_table_cells() {
        assert_eq!(escape_markdown(r"crate|name\x"), r"crate\|name\\x");
    }
}
