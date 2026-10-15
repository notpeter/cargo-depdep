use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn compares_the_working_lockfile_with_a_git_revision() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let repository =
        std::env::temp_dir().join(format!("cargo-depdep-cli-{}-{nonce}", std::process::id()));
    fs::create_dir(&repository).unwrap();

    git(&repository, &["init", "--initial-branch=main"]);
    fs::write(
        repository.join("Cargo.lock"),
        lockfile(&[
            ("foo", "1.0.0"),
            ("foo", "2.0.0"),
            ("removed", "4.0.0"),
            ("unchanged", "1.2.3"),
        ]),
    )
    .unwrap();
    fs::write(
        repository.join("Cargo.toml"),
        manifest(&["foo", "removed", "unchanged"]),
    )
    .unwrap();
    git(&repository, &["add", "Cargo.lock", "Cargo.toml"]);
    let revision = tree(&repository);

    fs::write(
        repository.join("Cargo.lock"),
        lockfile(&[
            ("added", "0.1.0"),
            ("foo", "2.0.0"),
            ("foo", "3.0.0"),
            ("unchanged", "1.2.3"),
        ]),
    )
    .unwrap();
    fs::write(
        repository.join("Cargo.toml"),
        manifest(&["added", "foo", "unchanged"]),
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_cargo-depdep"))
        .args(["depdep", "--rev", &revision])
        .current_dir(&repository)
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "| crate | old | new |\n\
         | --- | --- | --- |\n\
         | added |  | 0.1.0 |\n\
         | foo | 1.0.0, 2.0.0 | 2.0.0, 3.0.0 |\n\
         | removed | 4.0.0 |  |\n"
    );

    let pretty = Command::new(env!("CARGO_BIN_EXE_cargo-depdep"))
        .args(["depdep", "--pretty", "--rev", &revision])
        .current_dir(&repository)
        .output()
        .unwrap();

    assert!(
        pretty.status.success(),
        "command failed: {}",
        String::from_utf8_lossy(&pretty.stderr)
    );
    assert_eq!(
        String::from_utf8(pretty.stdout).unwrap(),
        "| crate   |          old |          new |\n\
         | :------ | -----------: | -----------: |\n\
         | added   |              |        0.1.0 |\n\
         | foo     | 1.0.0, 2.0.0 | 2.0.0, 3.0.0 |\n\
         | removed |        4.0.0 |              |\n"
    );

    fs::remove_dir_all(repository).unwrap();
}

#[test]
fn detects_ecosystems_and_allows_explicit_overrides() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let repository = std::env::temp_dir().join(format!(
        "cargo-depdep-detect-{}-{nonce}",
        std::process::id()
    ));
    let nested = repository.join("nested");
    fs::create_dir_all(&nested).unwrap();
    git(&repository, &["init", "--quiet"]);
    fs::write(repository.join("Cargo.toml"), manifest(&["foo"])).unwrap();
    fs::write(repository.join("Cargo.lock"), lockfile(&[("foo", "1.0.0")])).unwrap();
    fs::write(
        repository.join("package.json"),
        r#"{"dependencies":{"bar":"*"}}"#,
    )
    .unwrap();
    fs::write(
        repository.join("package-lock.json"),
        r#"{"lockfileVersion":3,"packages":{"node_modules/bar":{"version":"1.0.0"}}}"#,
    )
    .unwrap();
    git(&repository, &["add", "."]);
    let tree = Command::new("git")
        .arg("write-tree")
        .current_dir(&repository)
        .output()
        .unwrap();
    assert!(tree.status.success());
    let revision = String::from_utf8(tree.stdout).unwrap();
    fs::write(repository.join("Cargo.lock"), lockfile(&[("foo", "2.0.0")])).unwrap();
    fs::write(
        repository.join("package-lock.json"),
        r#"{"lockfileVersion":3,"packages":{"node_modules/bar":{"version":"2.0.0"}}}"#,
    )
    .unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_cargo-depdep"))
            .args(["depdep", "--rev", revision.trim()])
            .args(args)
            .current_dir(&nested)
            .output()
            .unwrap()
    };
    let rust = run(&["rust"]);
    let npm = run(&["npm"]);
    assert!(rust.status.success());
    assert!(npm.status.success());
    let rust_text = String::from_utf8(rust.stdout.clone()).unwrap();
    let npm_text = String::from_utf8(npm.stdout.clone()).unwrap();
    assert!(rust_text.contains("| foo | 1.0.0 | 2.0.0 |"));
    assert!(npm_text.contains("| bar | 1.0.0 | 2.0.0 |"));
    let both = run(&[]);
    assert!(both.status.success());
    assert_eq!(
        String::from_utf8(both.stdout).unwrap(),
        format!("## Rust\n\n{rust_text}\n## npm\n\n{npm_text}")
    );
    for alias in ["js", "ts", "node"] {
        let output = run(&[alias]);
        assert!(output.status.success());
        assert_eq!(output.stdout, npm.stdout);
    }
    fs::remove_file(repository.join("Cargo.toml")).unwrap();
    let only_npm = run(&[]);
    assert!(only_npm.status.success());
    assert_eq!(only_npm.stdout, npm.stdout);
    fs::write(repository.join("Cargo.toml"), manifest(&["foo"])).unwrap();
    fs::remove_file(repository.join("package.json")).unwrap();
    let only_rust = run(&[]);
    assert!(only_rust.status.success());
    assert_eq!(only_rust.stdout, rust.stdout);
    fs::remove_file(repository.join("Cargo.lock")).unwrap();
    let missing_lockfile = run(&[]);
    assert!(!missing_lockfile.status.success());
    assert!(
        String::from_utf8(missing_lockfile.stderr)
            .unwrap()
            .contains("Cargo.toml and Cargo.lock")
    );
    fs::remove_file(repository.join("Cargo.toml")).unwrap();
    let neither = run(&[]);
    assert!(!neither.status.success());
    assert!(
        String::from_utf8(neither.stderr)
            .unwrap()
            .contains("could not find Cargo.toml or package.json")
    );
    fs::remove_dir_all(repository).unwrap();
}

#[test]
fn missing_base_files_show_added_dependencies() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    for ecosystem in ["rust", "npm"] {
        for baseline in ["neither", "manifest", "lockfile"] {
            let repository = std::env::temp_dir().join(format!(
                "cargo-depdep-added-{}-{nonce}-{ecosystem}-{baseline}",
                std::process::id()
            ));
            let project = repository.join("xtask");
            fs::create_dir_all(&project).unwrap();
            git(&repository, &["init", "--quiet"]);
            let (manifest_name, manifest_text, lockfile_name, lockfile_text, label) = if ecosystem
                == "rust"
            {
                (
                    "Cargo.toml",
                    manifest(&["added"]),
                    "Cargo.lock",
                    lockfile(&[("added", "1.0.0"), ("transitive", "2.0.0")]),
                    "crate",
                )
            } else {
                (
                        "package.json",
                        r#"{"dependencies":{"added":"*"}}"#.into(),
                        "package-lock.json",
                        r#"{"lockfileVersion":3,"packages":{"node_modules/added":{"version":"1.0.0"},"node_modules/transitive":{"version":"2.0.0"}}}"#.into(),
                        "package",
                    )
            };
            if baseline == "manifest" {
                fs::write(project.join(manifest_name), &manifest_text).unwrap();
            } else if baseline == "lockfile" {
                fs::write(project.join(lockfile_name), &lockfile_text).unwrap();
            }
            git(&repository, &["add", "."]);
            let revision = tree(&repository);
            fs::write(project.join(manifest_name), &manifest_text).unwrap();
            fs::write(project.join(lockfile_name), &lockfile_text).unwrap();
            let run = |revision: &str, args: &[&str]| {
                Command::new(env!("CARGO_BIN_EXE_cargo-depdep"))
                    .args(["--rev", revision, ecosystem])
                    .args(args)
                    .current_dir(&project)
                    .output()
                    .unwrap()
            };
            for all in [false, true] {
                let output = run(&revision, if all { &["--all"] } else { &[] });
                assert!(
                    output.status.success(),
                    "{}",
                    String::from_utf8_lossy(&output.stderr)
                );
                if baseline == "lockfile" {
                    assert!(output.stdout.is_empty());
                    assert_eq!(String::from_utf8(output.stderr).unwrap(), "no changes\n");
                } else {
                    let mut expected = format!(
                        "| {label} | old | new |\n| --- | --- | --- |\n| added |  | 1.0.0 |\n"
                    );
                    if all {
                        expected.push_str("| transitive |  | 2.0.0 |\n");
                        assert!(output.stderr.is_empty());
                    } else {
                        assert!(
                            String::from_utf8_lossy(&output.stderr)
                                .contains("1 additional transitive dependency change")
                        );
                    }
                    assert_eq!(String::from_utf8(output.stdout).unwrap(), expected);
                }
            }
            let invalid = run("missing-revision", &[]);
            assert!(!invalid.status.success());
            assert!(
                String::from_utf8_lossy(&invalid.stderr)
                    .contains("could not read missing-revision:")
            );

            // Existing but malformed base files must still report parse errors.
            fs::write(
                project.join(lockfile_name),
                if ecosystem == "rust" {
                    "[[package]]\nname = \"broken\"\n"
                } else {
                    "{"
                },
            )
            .unwrap();
            git(&repository, &["add", "."]);
            let malformed = tree(&repository);
            fs::write(project.join(lockfile_name), &lockfile_text).unwrap();
            let output = run(&malformed, &[]);
            assert!(!output.status.success());
            assert!(String::from_utf8_lossy(&output.stderr).contains("could not parse"));
            fs::remove_dir_all(repository).unwrap();
        }
    }
}

fn tree(repository: &Path) -> String {
    let output = Command::new("git")
        .arg("write-tree")
        .current_dir(repository)
        .output()
        .unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn git(repository: &Path, arguments: &[&str]) {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(repository)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn lockfile(packages: &[(&str, &str)]) -> String {
    let mut contents =
        String::from("# This file is automatically @generated by Cargo.\nversion = 4\n");
    for (name, version) in packages {
        contents.push_str(&format!(
            "\n[[package]]\nname = \"{name}\"\nversion = \"{version}\"\n"
        ));
    }
    contents
}

fn manifest(dependencies: &[&str]) -> String {
    let mut contents =
        String::from("[package]\nname = \"test\"\nversion = \"0.0.0\"\n\n[dependencies]\n");
    for dependency in dependencies {
        contents.push_str(&format!("{dependency} = \"*\"\n"));
    }
    contents
}
