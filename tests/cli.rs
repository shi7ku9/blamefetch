use std::path::Path;
use std::process::{Command, Output};

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_blamefetch"))
}

fn git(dir: &Path, args: &[&str]) -> Output {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .expect("git binary is required for integration tests");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

fn make_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q"]);
    git(dir.path(), &["config", "user.name", "Test User"]);
    git(dir.path(), &["config", "user.email", "test@example.com"]);
    std::fs::write(dir.path().join("a.txt"), "hi\n").unwrap();
    git(dir.path(), &["add", "."]);
    git(
        dir.path(),
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-q",
            "-m",
            "test commit",
        ],
    );
    dir
}

#[cfg(unix)]
#[test]
fn golden_hermetic_output() {
    let repo = make_repo();
    // --config must be absolute: the test cwd is the temp repo, not the crate root.
    let minimal = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/minimal.toml");
    let run = || {
        bin()
            .arg("--no-git")
            .arg("--seed")
            .arg("42")
            .arg("--config")
            .arg(&minimal)
            .current_dir(repo.path())
            .output()
            .unwrap()
    };
    let out = run();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    let lines: Vec<&str> = stdout.lines().collect();
    assert!(lines[0].starts_with("commit "));
    assert_eq!(lines[0]["commit ".len()..].len(), 40);
    assert!(
        lines[0]["commit ".len()..]
            .chars()
            .all(|c| c.is_ascii_hexdigit())
    );
    assert_eq!(lines[1], "Author: Test User <test@example.com>");
    assert!(lines[2].is_empty());
    assert_eq!(lines[3], "feat: hermetic");
    // The fixture drives the co-author from a deterministic shell command.
    assert_eq!(lines.len(), 6);
    assert!(lines[4].is_empty());
    assert_eq!(lines[5], "Co-Authored-By: Bots 5.0 <bots@example.com>");

    // Determinism with a fixed seed.
    let out2 = run();
    assert_eq!(String::from_utf8(out2.stdout).unwrap(), stdout);
}

#[test]
fn uses_real_commit_in_repo() {
    let repo = make_repo();
    let head = String::from_utf8(git(repo.path(), &["rev-parse", "HEAD"]).stdout).unwrap();
    let head = head.trim().to_string();
    let out = bin().current_dir(repo.path()).output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        stdout.starts_with(&format!("commit {head}\n")),
        "should use the real HEAD hash"
    );
    assert!(stdout.contains("test commit"));
    assert!(stdout.contains("Author: Test User <test@example.com>"));
}

#[test]
fn blank_line_before_renders_between_groups() {
    let dir = tempfile::tempdir().unwrap();
    let cfg_path = dir.path().join("config.toml");
    std::fs::write(
        &cfg_path,
        r#"
[[co_authors]]
name = "First"
email = "first@x.com"

[[co_authors]]
blank_line_before = true
name = "Second"
email = "second@x.com"
"#,
    )
    .unwrap();
    let out = bin()
        .arg("--no-git")
        .arg("--seed")
        .arg("7")
        .arg("--config")
        .arg(&cfg_path)
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    let lines: Vec<&str> = stdout.lines().collect();
    let first = lines
        .iter()
        .position(|l| l.starts_with("Co-Authored-By: First"))
        .unwrap();
    let second = lines
        .iter()
        .position(|l| l.starts_with("Co-Authored-By: Second"))
        .unwrap();
    assert_eq!(second, first + 2, "blank line between the two trailers");
    assert_eq!(
        *lines.last().unwrap(),
        "Co-Authored-By: Second <second@x.com>",
        "no trailer after the last one"
    );
}

#[test]
fn verbose_notes_random_source_with_no_git() {
    let dir = tempfile::tempdir().unwrap();
    let out = bin()
        .arg("--no-git")
        .arg("-v")
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(out.status.success());
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("random commit data"), "stderr: {stderr}");
    assert!(stderr.contains("--no-git"), "stderr: {stderr}");
}

#[test]
fn verbose_notes_random_source_outside_repo() {
    let dir = tempfile::tempdir().unwrap();
    let out = bin().arg("-v").current_dir(dir.path()).output().unwrap();
    assert!(out.status.success());
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("random commit data"), "stderr: {stderr}");
    assert!(
        stderr.contains("not inside a git repository"),
        "stderr: {stderr}"
    );
}

#[test]
fn works_outside_a_repo() {
    let dir = tempfile::tempdir().unwrap();
    let out = bin().current_dir(dir.path()).output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    let first = stdout.lines().next().unwrap_or_default();
    assert!(first.starts_with("commit "));
    let hash = &first["commit ".len()..];
    assert_eq!(hash.len(), 40);
    assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn print_config_works() {
    let out = bin().arg("--print-config").output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("[[co_authors]]"));
    assert!(stdout.contains("kind = \"os\""));
    assert!(
        !stdout.contains("sample"),
        "defaults must not contain sample fixtures"
    );
}

#[cfg(unix)]
#[test]
fn print_config_does_not_run_commands() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("marker");
    let cfg_path = dir.path().join("config.toml");
    std::fs::write(
        &cfg_path,
        format!(
            r#"[[co_authors]]
name = "Bot"
email = "bot@x.com"

[co_authors.fields]
version = {{ command = "touch {}" }}
"#,
            marker.display()
        ),
    )
    .unwrap();

    // --print-config must never execute commands.
    let out = bin()
        .arg("--print-config")
        .arg("--config")
        .arg(&cfg_path)
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(!marker.exists(), "--print-config must not run commands");

    // A normal render does execute the configured command.
    let out = bin()
        .arg("--no-git")
        .arg("--config")
        .arg(&cfg_path)
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(
        marker.exists(),
        "render path must run the configured command"
    );
}

#[cfg(unix)]
#[test]
fn failing_command_without_fallback_skips_trailer() {
    let dir = tempfile::tempdir().unwrap();
    let cfg_path = dir.path().join("config.toml");
    std::fs::write(
        &cfg_path,
        r#"[[co_authors]]
name = "Bots {version}"
email = "bots@example.com"

[co_authors.fields]
version = { command = "false" }
"#,
    )
    .unwrap();
    let out = bin()
        .arg("--no-git")
        .arg("--config")
        .arg(&cfg_path)
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        !stdout.contains("Co-Authored-By:"),
        "a failed command without fallback must suppress the trailer:\n{stdout}"
    );
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("blamefetch: warning:"), "stderr: {stderr}");
}
