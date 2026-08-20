use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{self, Command};

const DEFAULT_REVISION: &str = "main";
const CARGO_MANIFEST: &str = "Cargo.toml";
const CARGO_LOCKFILE: &str = "Cargo.lock";
const NPM_MANIFEST: &str = "package.json";
const NPM_LOCKFILE: &str = "package-lock.json";

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
    let project = find_project(&current_dir, &repository_root, options.ecosystem)?;
    let lockfile_path = project
        .lockfile
        .strip_prefix(&repository_root)
        .expect("the lockfile search stays inside the repository");
    let manifest_path = project
        .manifest
        .strip_prefix(&repository_root)
        .expect("the manifest search stays inside the repository");

    let old_lockfile = file_at_revision(&repository_root, &revision, lockfile_path)?;
    let new_lockfile = fs::read_to_string(&project.lockfile).map_err(|source| Error::Io {
        context: format!("could not read {}", project.lockfile.display()),
        source,
    })?;
    let old_manifest = file_at_revision(&repository_root, &revision, manifest_path)?;
    let new_manifest = fs::read_to_string(&project.manifest).map_err(|source| Error::Io {
        context: format!("could not read {}", project.manifest.display()),
        source,
    })?;

    let old_packages = options
        .ecosystem
        .parse_lockfile(&old_lockfile)
        .map_err(|message| Error::File {
            label: format!("{revision}:{}", lockfile_path.display()),
            message,
        })?;
    let new_packages = options
        .ecosystem
        .parse_lockfile(&new_lockfile)
        .map_err(|message| Error::File {
            label: project.lockfile.display().to_string(),
            message,
        })?;
    let old_direct = options
        .ecosystem
        .parse_manifest(&old_manifest)
        .map_err(|message| Error::File {
            label: format!("{revision}:{}", manifest_path.display()),
            message,
        })?;
    let new_direct = options
        .ecosystem
        .parse_manifest(&new_manifest)
        .map_err(|message| Error::File {
            label: project.manifest.display().to_string(),
            message,
        })?;
    let direct = old_direct.union(&new_direct).cloned().collect();

    print_diff(
        &old_packages,
        &new_packages,
        &direct,
        options.transitive,
        options.pretty,
        options.ecosystem.package_label(),
    );
    Ok(())
}

#[derive(Clone, Copy)]
enum Ecosystem {
    Rust,
    Npm,
}

impl Ecosystem {
    fn parse_lockfile(self, contents: &str) -> Result<Packages, String> {
        match self {
            Self::Rust => parse_cargo_lockfile(contents),
            Self::Npm => parse_npm_lockfile(contents),
        }
    }

    fn parse_manifest(self, contents: &str) -> Result<BTreeSet<String>, String> {
        match self {
            Self::Rust => parse_cargo_manifest(contents),
            Self::Npm => parse_npm_manifest(contents),
        }
    }

    fn package_label(self) -> &'static str {
        match self {
            Self::Rust => "crate",
            Self::Npm => "package",
        }
    }

    fn project_files(self) -> &'static str {
        match self {
            Self::Rust => "Cargo.toml and Cargo.lock",
            Self::Npm => "package.json and package-lock.json",
        }
    }
}

struct Options {
    ecosystem: Ecosystem,
    revision: Option<String>,
    transitive: bool,
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
    let mut transitive = false;
    let mut ecosystem = None;

    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("-h" | "--help") => {
                println!(
                    "Compare dependency lockfiles with another Git revision.\n\n\
                     Usage: cargo depdep [OPTIONS] [ECOSYSTEM]\n\n\
                     Arguments:\n  [ECOSYSTEM]  Package ecosystem: rust or npm [default: rust]\n\n\
                     Options:\n  \
                       --rev <REV>  Git rev to compare against [default: main or repo default branch]\n  \
                       --transitive  Include transitive dependency changes\n  \
                       --pretty  Align the columns for a nicely formatted ASCII table\n  \
                   -h, --help    Print help"
                );
                process::exit(0);
            }
            Some("--pretty") => pretty = true,
            Some("--transitive") => transitive = true,
            Some("rust" | "npm") => {
                if ecosystem.is_some() {
                    return Err(Error::Usage("expected at most one ecosystem".into()));
                }
                ecosystem = Some(if arg == "rust" {
                    Ecosystem::Rust
                } else {
                    Ecosystem::Npm
                });
            }
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

    Ok(Options {
        ecosystem: ecosystem.unwrap_or(Ecosystem::Rust),
        revision,
        transitive,
        pretty,
    })
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

struct Project {
    manifest: PathBuf,
    lockfile: PathBuf,
}

fn find_project(
    current_dir: &Path,
    repository_root: &Path,
    ecosystem: Ecosystem,
) -> Result<Project, Error> {
    let (manifest_name, lockfile_name) = match ecosystem {
        Ecosystem::Rust => (CARGO_MANIFEST, CARGO_LOCKFILE),
        Ecosystem::Npm => (NPM_MANIFEST, NPM_LOCKFILE),
    };
    let manifest = find_file(current_dir, repository_root, manifest_name);
    let lockfile = find_file(current_dir, repository_root, lockfile_name);

    match (manifest, lockfile) {
        (Some(manifest), Some(lockfile)) => Ok(Project { manifest, lockfile }),
        _ => Err(Error::Usage(format!(
            "could not find {} between {} and {}",
            ecosystem.project_files(),
            current_dir.display(),
            repository_root.display()
        ))),
    }
}

fn find_file(current_dir: &Path, repository_root: &Path, name: &str) -> Option<PathBuf> {
    let mut directory = current_dir;

    loop {
        let candidate = directory.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
        if directory == repository_root {
            break;
        }
        directory = directory.parent()?;
    }

    None
}

fn file_at_revision(
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

fn parse_cargo_lockfile(contents: &str) -> Result<Packages, String> {
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
    if let Some(inner) = value
        .strip_prefix('\'')
        .and_then(|value| value.strip_suffix('\''))
    {
        return Ok(inner.to_string());
    }
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

fn parse_cargo_manifest(contents: &str) -> Result<BTreeSet<String>, String> {
    let mut dependencies = BTreeSet::new();
    let mut section = CargoManifestSection::Other;

    for (index, line) in contents.lines().enumerate() {
        let line_number = index + 1;
        let line = strip_toml_comment(line).trim();
        if line.is_empty() {
            continue;
        }

        if line.starts_with('[') && line.ends_with(']') {
            section = cargo_manifest_section(line, line_number)?;
            if let CargoManifestSection::Dependency { resolved } = &section {
                dependencies.insert(resolved.clone());
            }
            continue;
        }

        let Some((key, value)) = toml_assignment(line) else {
            continue;
        };
        match &mut section {
            CargoManifestSection::Dependencies => {
                let keys = split_toml_key(key, line_number)?;
                let alias = keys
                    .first()
                    .ok_or_else(|| format!("line {line_number}: expected a dependency name"))?;
                let name = inline_toml_string_field(value, "package", line_number)?
                    .unwrap_or_else(|| alias.clone());
                dependencies.insert(name);
            }
            CargoManifestSection::Dependency { resolved } => {
                if parse_toml_key(key, line_number)? == "package" {
                    dependencies.remove(resolved);
                    let name = parse_toml_string(value.trim(), line_number)?;
                    dependencies.insert(name.clone());
                    *resolved = name;
                }
            }
            CargoManifestSection::Other => {}
        }
    }

    Ok(dependencies)
}

enum CargoManifestSection {
    Other,
    Dependencies,
    Dependency { resolved: String },
}

fn cargo_manifest_section(
    header: &str,
    line_number: usize,
) -> Result<CargoManifestSection, String> {
    if header.starts_with("[[") {
        return Ok(CargoManifestSection::Other);
    }
    let keys = split_toml_key(&header[1..header.len() - 1], line_number)?;
    let dependency_group = |key: &str| {
        matches!(
            key,
            "dependencies" | "dev-dependencies" | "build-dependencies"
        )
    };

    let group_index = if keys.first().is_some_and(|key| dependency_group(key)) {
        Some(0)
    } else if keys.first().is_some_and(|key| key == "target") {
        keys.iter().position(|key| dependency_group(key))
    } else {
        None
    };
    let Some(group_index) = group_index else {
        return Ok(CargoManifestSection::Other);
    };

    match keys.len().saturating_sub(group_index) {
        1 => Ok(CargoManifestSection::Dependencies),
        2 => {
            let alias = keys[group_index + 1].clone();
            Ok(CargoManifestSection::Dependency { resolved: alias })
        }
        _ => Ok(CargoManifestSection::Other),
    }
}

fn strip_toml_comment(line: &str) -> &str {
    let mut quote = None;
    let mut escaped = false;
    for (offset, character) in line.char_indices() {
        match quote {
            Some('"') if escaped => escaped = false,
            Some('"') if character == '\\' => escaped = true,
            Some(expected) if character == expected => quote = None,
            Some(_) => {}
            None if matches!(character, '"' | '\'') => quote = Some(character),
            None if character == '#' => return &line[..offset],
            None => {}
        }
    }
    line
}

fn toml_assignment(line: &str) -> Option<(&str, &str)> {
    let mut quote = None;
    let mut escaped = false;
    for (offset, character) in line.char_indices() {
        match quote {
            Some('"') if escaped => escaped = false,
            Some('"') if character == '\\' => escaped = true,
            Some(expected) if character == expected => quote = None,
            Some(_) => {}
            None if matches!(character, '"' | '\'') => quote = Some(character),
            None if character == '=' => return Some((&line[..offset], &line[offset + 1..])),
            None => {}
        }
    }
    None
}

fn split_toml_key(key: &str, line_number: usize) -> Result<Vec<String>, String> {
    let mut keys = Vec::new();
    let mut quote = None;
    let mut escaped = false;
    let mut start = 0;

    for (offset, character) in key.char_indices() {
        match quote {
            Some('"') if escaped => escaped = false,
            Some('"') if character == '\\' => escaped = true,
            Some(expected) if character == expected => quote = None,
            Some(_) => {}
            None if matches!(character, '"' | '\'') => quote = Some(character),
            None if character == '.' => {
                keys.push(parse_toml_key(&key[start..offset], line_number)?);
                start = offset + 1;
            }
            None => {}
        }
    }
    if quote.is_some() {
        return Err(format!("line {line_number}: unterminated quoted key"));
    }
    keys.push(parse_toml_key(&key[start..], line_number)?);
    Ok(keys)
}

fn parse_toml_key(key: &str, line_number: usize) -> Result<String, String> {
    let key = key.trim();
    if matches!(key.as_bytes().first(), Some(b'"' | b'\'')) {
        parse_toml_string(key, line_number)
    } else if key.is_empty() {
        Err(format!("line {line_number}: expected a TOML key"))
    } else {
        Ok(key.to_string())
    }
}

fn inline_toml_string_field(
    value: &str,
    field: &str,
    line_number: usize,
) -> Result<Option<String>, String> {
    let value = value.trim();
    let Some(value) = value
        .strip_prefix('{')
        .and_then(|value| value.strip_suffix('}'))
    else {
        return Ok(None);
    };

    let mut quote = None;
    let mut escaped = false;
    let mut nesting = 0_u32;
    let mut start = 0;
    let mut fields = Vec::new();
    for (offset, character) in value.char_indices() {
        match quote {
            Some('"') if escaped => escaped = false,
            Some('"') if character == '\\' => escaped = true,
            Some(expected) if character == expected => quote = None,
            Some(_) => {}
            None if matches!(character, '"' | '\'') => quote = Some(character),
            None if matches!(character, '[' | '{') => nesting += 1,
            None if matches!(character, ']' | '}') => nesting = nesting.saturating_sub(1),
            None if character == ',' && nesting == 0 => {
                fields.push(&value[start..offset]);
                start = offset + 1;
            }
            None => {}
        }
    }
    fields.push(&value[start..]);

    for assignment in fields {
        let Some((key, value)) = toml_assignment(assignment) else {
            continue;
        };
        if parse_toml_key(key, line_number)? == field {
            return parse_toml_string(value.trim(), line_number).map(Some);
        }
    }
    Ok(None)
}

fn parse_npm_manifest(contents: &str) -> Result<BTreeSet<String>, String> {
    let json = parse_json(contents)?;
    let root = json
        .as_object()
        .ok_or_else(|| "expected the manifest to contain a JSON object".to_string())?;
    let mut dependencies = BTreeSet::new();

    for field in [
        "dependencies",
        "devDependencies",
        "optionalDependencies",
        "peerDependencies",
    ] {
        let Some(value) = root.get(field) else {
            continue;
        };
        let object = value
            .as_object()
            .ok_or_else(|| format!("expected {field:?} to be a JSON object"))?;
        dependencies.extend(object.keys().cloned());
    }

    Ok(dependencies)
}

fn parse_npm_lockfile(contents: &str) -> Result<Packages, String> {
    let json = parse_json(contents)?;
    let root = json
        .as_object()
        .ok_or_else(|| "expected the lockfile to contain a JSON object".to_string())?;
    let mut packages = Packages::new();

    if let Some(records) = root.get("packages") {
        let records = records
            .as_object()
            .ok_or_else(|| "expected \"packages\" to be a JSON object".to_string())?;
        for (path, value) in records {
            if path.is_empty() {
                continue;
            }
            let record = value
                .as_object()
                .ok_or_else(|| format!("expected package {path:?} to be a JSON object"))?;
            let Some(version) = json_string_field(record, "version")? else {
                // Linked workspaces have no version at their node_modules entry.
                continue;
            };
            let name = match json_string_field(record, "name")? {
                Some(name) => name,
                None => match npm_package_name(path) {
                    Some(name) => name,
                    None => continue,
                },
            };
            packages
                .entry(name.to_string())
                .or_default()
                .insert(version.to_string());
        }
        return Ok(packages);
    }

    // package-lock v1 stores dependencies as a recursively nested tree.
    if let Some(dependencies) = root.get("dependencies") {
        let dependencies = dependencies
            .as_object()
            .ok_or_else(|| "expected \"dependencies\" to be a JSON object".to_string())?;
        add_npm_dependency_tree(dependencies, &mut packages)?;
    }

    Ok(packages)
}

fn add_npm_dependency_tree(
    dependencies: &BTreeMap<String, JsonValue>,
    packages: &mut Packages,
) -> Result<(), String> {
    for (name, value) in dependencies {
        let dependency = value
            .as_object()
            .ok_or_else(|| format!("expected dependency {name:?} to be a JSON object"))?;
        let version = json_string_field(dependency, "version")?
            .ok_or_else(|| format!("dependency {name:?} does not have a version"))?;
        packages
            .entry(name.clone())
            .or_default()
            .insert(version.to_string());

        if let Some(nested) = dependency.get("dependencies") {
            let nested = nested.as_object().ok_or_else(|| {
                format!("expected nested dependencies for {name:?} to be a JSON object")
            })?;
            add_npm_dependency_tree(nested, packages)?;
        }
    }
    Ok(())
}

fn npm_package_name(path: &str) -> Option<&str> {
    let (_, name) = path.rsplit_once("node_modules/")?;
    (!name.is_empty()).then_some(name)
}

fn json_string_field<'a>(
    object: &'a BTreeMap<String, JsonValue>,
    field: &str,
) -> Result<Option<&'a str>, String> {
    match object.get(field) {
        Some(JsonValue::String(value)) => Ok(Some(value)),
        Some(_) => Err(format!("expected {field:?} to be a JSON string")),
        None => Ok(None),
    }
}

enum JsonValue {
    Object(BTreeMap<String, JsonValue>),
    String(String),
    Other,
}

impl JsonValue {
    fn as_object(&self) -> Option<&BTreeMap<String, Self>> {
        match self {
            Self::Object(object) => Some(object),
            _ => None,
        }
    }
}

fn parse_json(input: &str) -> Result<JsonValue, String> {
    let mut parser = JsonParser { input, offset: 0 };
    let value = parser.parse_value()?;
    parser.skip_whitespace();
    if parser.offset != input.len() {
        return Err(parser.error("unexpected trailing content"));
    }
    Ok(value)
}

struct JsonParser<'a> {
    input: &'a str,
    offset: usize,
}

impl JsonParser<'_> {
    fn parse_value(&mut self) -> Result<JsonValue, String> {
        self.skip_whitespace();
        match self.peek() {
            Some(b'{') => self.parse_object(),
            Some(b'[') => self.parse_array(),
            Some(b'"') => self.parse_string().map(JsonValue::String),
            Some(b't') => self.parse_literal("true"),
            Some(b'f') => self.parse_literal("false"),
            Some(b'n') => self.parse_literal("null"),
            Some(b'-' | b'0'..=b'9') => self.parse_number(),
            Some(_) => Err(self.error("expected a JSON value")),
            None => Err(self.error("unexpected end of input")),
        }
    }

    fn parse_object(&mut self) -> Result<JsonValue, String> {
        self.expect(b'{')?;
        let mut object = BTreeMap::new();
        self.skip_whitespace();
        if self.consume(b'}') {
            return Ok(JsonValue::Object(object));
        }

        loop {
            self.skip_whitespace();
            let key = self.parse_string()?;
            self.skip_whitespace();
            self.expect(b':')?;
            let value = self.parse_value()?;
            object.insert(key, value);
            self.skip_whitespace();
            if self.consume(b'}') {
                break;
            }
            self.expect(b',')?;
        }

        Ok(JsonValue::Object(object))
    }

    fn parse_array(&mut self) -> Result<JsonValue, String> {
        self.expect(b'[')?;
        self.skip_whitespace();
        if self.consume(b']') {
            return Ok(JsonValue::Other);
        }

        loop {
            self.parse_value()?;
            self.skip_whitespace();
            if self.consume(b']') {
                break;
            }
            self.expect(b',')?;
        }

        Ok(JsonValue::Other)
    }

    fn parse_string(&mut self) -> Result<String, String> {
        self.expect(b'"')?;
        let mut value = String::new();

        loop {
            let byte = self
                .peek()
                .ok_or_else(|| self.error("unterminated JSON string"))?;
            match byte {
                b'"' => {
                    self.offset += 1;
                    return Ok(value);
                }
                b'\\' => {
                    self.offset += 1;
                    let escaped = self
                        .peek()
                        .ok_or_else(|| self.error("incomplete JSON string escape"))?;
                    self.offset += 1;
                    match escaped {
                        b'"' => value.push('"'),
                        b'\\' => value.push('\\'),
                        b'/' => value.push('/'),
                        b'b' => value.push('\u{0008}'),
                        b'f' => value.push('\u{000c}'),
                        b'n' => value.push('\n'),
                        b'r' => value.push('\r'),
                        b't' => value.push('\t'),
                        b'u' => value.push(self.parse_unicode_escape()?),
                        _ => return Err(self.error("unsupported JSON string escape")),
                    }
                }
                0..=0x1f => return Err(self.error("control character in JSON string")),
                0x20..=0x7f => {
                    value.push(char::from(byte));
                    self.offset += 1;
                }
                _ => {
                    let character = self.input[self.offset..]
                        .chars()
                        .next()
                        .expect("the offset is on a UTF-8 character boundary");
                    value.push(character);
                    self.offset += character.len_utf8();
                }
            }
        }
    }

    fn parse_unicode_escape(&mut self) -> Result<char, String> {
        let first = self.parse_hex_quad()?;
        let code_point = if (0xd800..=0xdbff).contains(&first) {
            if !self.input[self.offset..].starts_with("\\u") {
                return Err(self.error("high surrogate without a low surrogate"));
            }
            self.offset += 2;
            let second = self.parse_hex_quad()?;
            if !(0xdc00..=0xdfff).contains(&second) {
                return Err(self.error("invalid low surrogate"));
            }
            0x10000 + (u32::from(first - 0xd800) << 10) + u32::from(second - 0xdc00)
        } else if (0xdc00..=0xdfff).contains(&first) {
            return Err(self.error("low surrogate without a high surrogate"));
        } else {
            u32::from(first)
        };
        char::from_u32(code_point).ok_or_else(|| self.error("invalid Unicode escape"))
    }

    fn parse_hex_quad(&mut self) -> Result<u16, String> {
        if self.offset + 4 > self.input.len() {
            return Err(self.error("incomplete Unicode escape"));
        }
        let mut value = 0_u16;
        for _ in 0..4 {
            let digit = self.input.as_bytes()[self.offset];
            self.offset += 1;
            let digit = match digit {
                b'0'..=b'9' => u16::from(digit - b'0'),
                b'a'..=b'f' => u16::from(digit - b'a' + 10),
                b'A'..=b'F' => u16::from(digit - b'A' + 10),
                _ => return Err(self.error("invalid Unicode escape")),
            };
            value = value * 16 + digit;
        }
        Ok(value)
    }

    fn parse_literal(&mut self, literal: &str) -> Result<JsonValue, String> {
        if !self.input[self.offset..].starts_with(literal) {
            return Err(self.error("invalid JSON literal"));
        }
        self.offset += literal.len();
        Ok(JsonValue::Other)
    }

    fn parse_number(&mut self) -> Result<JsonValue, String> {
        self.consume(b'-');
        if self.consume(b'0') {
            // A leading zero is only valid when it is the entire integer part.
        } else {
            self.consume_digits(true)?;
        }
        if self.consume(b'.') {
            self.consume_digits(true)?;
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.offset += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.offset += 1;
            }
            self.consume_digits(true)?;
        }
        Ok(JsonValue::Other)
    }

    fn consume_digits(&mut self, require_one: bool) -> Result<(), String> {
        let start = self.offset;
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.offset += 1;
        }
        if require_one && self.offset == start {
            return Err(self.error("expected a digit"));
        }
        Ok(())
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\n' | b'\r' | b'\t')) {
            self.offset += 1;
        }
    }

    fn expect(&mut self, expected: u8) -> Result<(), String> {
        if self.consume(expected) {
            Ok(())
        } else {
            Err(self.error(&format!("expected {:?}", char::from(expected))))
        }
    }

    fn consume(&mut self, expected: u8) -> bool {
        if self.peek() == Some(expected) {
            self.offset += 1;
            true
        } else {
            false
        }
    }

    fn peek(&self) -> Option<u8> {
        self.input.as_bytes().get(self.offset).copied()
    }

    fn error(&self, message: &str) -> String {
        format!("{message} at byte {}", self.offset)
    }
}

fn print_diff(
    old: &Packages,
    new: &Packages,
    direct: &BTreeSet<String>,
    include_transitive: bool,
    pretty: bool,
    package_label: &str,
) {
    let mut rows = Vec::new();
    let mut hidden = 0;
    for name in old.keys().chain(new.keys()).collect::<BTreeSet<_>>() {
        if old.get(name) == new.get(name) {
            continue;
        }
        if !include_transitive && !direct.contains(name) {
            hidden += 1;
            continue;
        }

        rows.push([
            escape_markdown(name),
            versions(old.get(name)),
            versions(new.get(name)),
        ]);
    }

    if rows.is_empty() && hidden == 0 {
        eprintln!("no changes");
    }

    if pretty && !rows.is_empty() {
        print_pretty(&rows, package_label);
    } else if !rows.is_empty() {
        print_plain(&rows, package_label);
    }

    if hidden != 0 {
        let (dependency, pronoun) = if hidden == 1 {
            ("dependency change", "it")
        } else {
            ("dependency changes", "them")
        };
        eprintln!(
            "{hidden} additional transitive {dependency} not shown; use --transitive to show {pronoun}"
        );
    }
}

fn print_plain(rows: &[[String; 3]], package_label: &str) {
    println!("| {package_label} | old | new |");
    println!("| --- | --- | --- |");

    for row in rows {
        println!("| {} | {} | {} |", row[0], row[1], row[2]);
    }
}

fn print_pretty(rows: &[[String; 3]], package_label: &str) {
    let headers = [package_label, "old", "new"];
    let mut widths = headers.map(|header| header.chars().count());
    for row in rows {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.chars().count());
        }
    }

    // The package name stays left-aligned; the version columns are right-aligned.
    println!(
        "| {:<crate$} | {:>old$} | {:>new$} |",
        headers[0],
        headers[1],
        headers[2],
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
    File {
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
            Self::File { label, message } => {
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
        let packages = parse_cargo_lockfile(
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
        let packages = parse_cargo_lockfile(
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
