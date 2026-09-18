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
    git(
        &repository,
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "old lockfile",
        ],
    );

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
        .args(["depdep", "--rev", "HEAD"])
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
        .args(["depdep", "--pretty", "--rev", "HEAD"])
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
