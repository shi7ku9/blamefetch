use std::collections::BTreeMap;
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

/// A repo with `count` commits (one empty-ish file change per commit), all
/// authored by "Test User".
fn make_commits_repo(count: usize) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q"]);
    git(dir.path(), &["config", "user.name", "Test User"]);
    git(dir.path(), &["config", "user.email", "test@example.com"]);
    for i in 0..count {
        let msg = format!("commit {i}");
        std::fs::write(dir.path().join("f.txt"), format!("{i}\n")).unwrap();
        git(dir.path(), &["add", "."]);
        git(
            dir.path(),
            &["-c", "commit.gpgsign=false", "commit", "-q", "-m", &msg],
        );
    }
    dir
}

#[cfg(unix)]
#[test]
fn golden_hermetic_output() {
    let repo = make_repo();
    // --config must be absolute: the test cwd is the temp repo, not the crate root.
    let minimal = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/minimal.json");
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
    // The random date must look like git's default `%ad` ("Thu Aug 6
    // 05:32:10 2026 +0800"): weekday, month, unpadded day, time, year, offset.
    assert!(lines[2].starts_with("Date:   "));
    let date = &lines[2]["Date:   ".len()..];
    let parts: Vec<&str> = date.split_whitespace().collect();
    assert_eq!(parts.len(), 6, "unexpected date format: {date:?}");
    assert!(parts[0].chars().all(|c| c.is_alphabetic()) && parts[0].len() == 3);
    assert!(parts[1].chars().all(|c| c.is_alphabetic()) && parts[1].len() == 3);
    assert!(parts[2].parse::<u32>().is_ok() && parts[2].len() <= 2);
    assert!(parts[3].len() == 8 && parts[3].chars().nth(2) == Some(':'));
    assert!(parts[4].parse::<u32>().is_ok() && parts[4].len() == 4);
    assert!(
        (parts[5].starts_with('+') || parts[5].starts_with('-'))
            && parts[5].len() == 5
            && parts[5][1..].chars().all(|c| c.is_ascii_digit())
    );
    assert!(lines[3].is_empty());
    assert_eq!(lines[4], "    feat: hermetic");
    // The fixture drives the co-author from a deterministic shell command.
    assert_eq!(lines.len(), 7);
    assert_eq!(lines[5], "    ", "blank separator line indented like git");
    assert_eq!(lines[6], "    Co-Authored-By: Bots 5.0 <bots@example.com>");

    // Determinism with a fixed seed: everything is byte-identical except the
    // Date line, which is relative to the wall clock and advances between runs.
    let out2 = run();
    let stdout2 = String::from_utf8(out2.stdout).unwrap();
    let mask_date = |s: &str| -> String {
        s.lines()
            .map(|l| {
                if l.starts_with("Date:   ") {
                    "Date: <masked>"
                } else {
                    l
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert_eq!(mask_date(&stdout2), mask_date(&stdout));
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
    assert!(
        stdout.contains("Date: "),
        "real commit must show its author date:\n{stdout}"
    );
}

#[test]
fn commit_flag_full_hash_uses_commit() {
    let repo = make_repo();
    let head = String::from_utf8(git(repo.path(), &["rev-parse", "HEAD"]).stdout).unwrap();
    let head = head.trim().to_string();
    let out = bin()
        .arg("--commit")
        .arg(&head)
        .current_dir(repo.path())
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        stdout.starts_with(&format!("commit {head}\n")),
        "should use the selected commit hash:\n{stdout}"
    );
    assert!(stdout.contains("test commit"));
    assert!(stdout.contains("Author: Test User <test@example.com>"));
}

#[test]
fn commit_flag_short_uppercase_prefix_matches() {
    let repo = make_repo();
    let head = String::from_utf8(git(repo.path(), &["rev-parse", "HEAD"]).stdout).unwrap();
    let head = head.trim().to_string();
    // Include at least one letter in the prefix so the uppercasing is a real
    // transformation; an all-digit prefix would make the case fold vacuous.
    // A 40-digit hash is theoretically possible, so fall back to a 39-char
    // prefix (still a unique match) instead of assuming a letter exists.
    let letter_idx = head
        .find(|c: char| c.is_ascii_alphabetic())
        .unwrap_or(head.len() - 1);
    let out = bin()
        .arg("--commit")
        .arg(head[..=letter_idx].to_uppercase())
        .current_dir(repo.path())
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        stdout.starts_with(&format!("commit {head}\n")),
        "short uppercase prefix should match the HEAD hash:\n{stdout}"
    );
}

#[test]
fn commit_flag_multiple_matches_errors() {
    // 17 commits over 16 possible first hex digits guarantee by the
    // pigeonhole principle that at least one one-character prefix matches
    // two or more hashes, so the error path is deterministic.
    let repo = make_commits_repo(17);
    let all = String::from_utf8(git(repo.path(), &["rev-list", "--all"]).stdout).unwrap();
    let hashes: Vec<&str> = all
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    assert!(hashes.len() >= 17);
    let mut groups: BTreeMap<char, Vec<String>> = BTreeMap::new();
    for hash in &hashes {
        groups
            .entry(hash.chars().next().unwrap())
            .or_default()
            .push(hash.to_string());
    }
    let (_, candidates) = groups
        .iter()
        .find(|(_, v)| v.len() >= 2)
        .expect("17 hashes must share at least one first hex digit");
    let prefix = candidates[0][..1].to_string();

    let out = bin()
        .arg("--commit")
        .arg(&prefix)
        .current_dir(repo.path())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(
        stderr.contains("matches") && stderr.contains("commits:"),
        "multiple-match error expected:\n{stderr}"
    );
    for hash in candidates {
        assert!(
            stderr.contains(hash),
            "candidate {hash} must be listed in the error:\n{stderr}"
        );
    }
}

#[test]
fn commit_flag_no_match_errors() {
    let repo = make_repo();
    let head = String::from_utf8(git(repo.path(), &["rev-parse", "HEAD"]).stdout).unwrap();
    let head = head.trim();
    // Deterministic no-match: rotate HEAD's first hex digit to a different
    // one, which cannot be the first digit of the only commit in the repo.
    let first = head.chars().next().unwrap().to_digit(16).unwrap();
    let other = char::from_digit((first + 1) % 16, 16).unwrap();
    let out = bin()
        .arg("--commit")
        .arg(other.to_string())
        .current_dir(repo.path())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(
        stderr.contains("does not match any commit"),
        "no-match error expected:\n{stderr}"
    );
}

#[test]
fn commit_flag_conflicts_with_no_git() {
    let dir = tempfile::tempdir().unwrap();
    let out = bin()
        .arg("--commit")
        .arg("abc")
        .arg("--no-git")
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(
        stderr.contains("cannot be used with"),
        "--commit must conflict with --no-git:\n{stderr}"
    );
}

#[test]
fn commit_flag_conflicts_with_hash() {
    let dir = tempfile::tempdir().unwrap();
    let out = bin()
        .arg("--commit")
        .arg("abc")
        .arg("--hash")
        .arg("def")
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(
        stderr.contains("cannot be used with"),
        "--commit must conflict with --hash:\n{stderr}"
    );
}

#[test]
fn commit_flag_empty_prefix_errors() {
    let repo = make_repo();
    let out = bin()
        .arg("--commit")
        .arg("")
        .current_dir(repo.path())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(
        stderr.contains("non-empty hexadecimal"),
        "empty prefix must be rejected with a hex hint:\n{stderr}"
    );
}

#[test]
fn commit_flag_non_hex_prefix_errors() {
    let repo = make_repo();
    let out = bin()
        .arg("--commit")
        .arg("xyz")
        .current_dir(repo.path())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(
        stderr.contains("non-empty hexadecimal"),
        "non-hex prefix must be rejected with a hex hint:\n{stderr}"
    );
}

#[test]
fn commit_flag_outside_repo_errors() {
    let dir = tempfile::tempdir().unwrap();
    let out = bin()
        .arg("--commit")
        .arg("abc")
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(
        stderr.contains("requires being inside a Git repository"),
        "--commit outside a repo must explain the requirement:\n{stderr}"
    );
}

#[test]
fn commit_flag_empty_repo_errors() {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q"]);
    let out = bin()
        .arg("--commit")
        .arg("abc")
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(
        stderr.contains("does not match any commit"),
        "an empty repo must report no match:\n{stderr}"
    );
}

#[test]
fn commit_flag_author_and_message_overrides() {
    let repo = make_repo();
    let head = String::from_utf8(git(repo.path(), &["rev-parse", "HEAD"]).stdout).unwrap();
    let head = head.trim().to_string();
    let out = bin()
        .arg("--commit")
        .arg(&head)
        .arg("--author")
        .arg("CLI User")
        .arg("--email")
        .arg("cli@x.com")
        .arg("--message")
        .arg("forced message")
        .current_dir(repo.path())
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        stdout.starts_with(&format!("commit {head}\n")),
        "selected commit hash must remain:\n{stdout}"
    );
    assert!(stdout.contains("Author: CLI User <cli@x.com>"), "{stdout}");
    assert!(stdout.contains("    forced message"), "{stdout}");
}

#[test]
fn commit_flag_unicode_prefix_errors() {
    let repo = make_repo();
    let out = bin()
        .arg("--commit")
        .arg("你")
        .current_dir(repo.path())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(
        stderr.contains("not a valid commit prefix"),
        "a non-hex prefix must be rejected as invalid:\n{stderr}"
    );
}

#[test]
fn commit_flag_multiline_message_renders() {
    let repo = tempfile::tempdir().unwrap();
    git(repo.path(), &["init", "-q"]);
    git(repo.path(), &["config", "user.name", "Test User"]);
    git(repo.path(), &["config", "user.email", "test@example.com"]);
    std::fs::write(repo.path().join("a.txt"), "hi\n").unwrap();
    git(repo.path(), &["add", "."]);
    git(
        repo.path(),
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-q",
            "-m",
            "first line",
            "-m",
            "second line",
        ],
    );
    let head = String::from_utf8(git(repo.path(), &["rev-parse", "HEAD"]).stdout).unwrap();
    let out = bin()
        .arg("--commit")
        .arg(head.trim())
        .current_dir(repo.path())
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("    first line"), "{stdout}");
    assert!(stdout.contains("    second line"), "{stdout}");
}

#[test]
fn commit_flag_percent_message_passes_through() {
    let repo = tempfile::tempdir().unwrap();
    git(repo.path(), &["init", "-q"]);
    git(repo.path(), &["config", "user.name", "Test User"]);
    git(repo.path(), &["config", "user.email", "test@example.com"]);
    std::fs::write(repo.path().join("a.txt"), "hi\n").unwrap();
    git(repo.path(), &["add", "."]);
    git(
        repo.path(),
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-q",
            "-m",
            "%s %B %n stays literal",
        ],
    );
    let head = String::from_utf8(git(repo.path(), &["rev-parse", "HEAD"]).stdout).unwrap();
    let out = bin()
        .arg("--commit")
        .arg(head.trim())
        .current_dir(repo.path())
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        stdout.contains("    %s %B %n stays literal"),
        "message must be rendered verbatim, never re-expanded:\n{stdout}"
    );
}

#[test]
fn commit_flag_empty_message_commit_renders() {
    let repo = tempfile::tempdir().unwrap();
    git(repo.path(), &["init", "-q"]);
    git(repo.path(), &["config", "user.name", "Test User"]);
    git(repo.path(), &["config", "user.email", "test@example.com"]);
    std::fs::write(repo.path().join("a.txt"), "hi\n").unwrap();
    git(repo.path(), &["add", "."]);
    git(
        repo.path(),
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-q",
            "--allow-empty-message",
            "-m",
            "",
        ],
    );
    let head = String::from_utf8(git(repo.path(), &["rev-parse", "HEAD"]).stdout).unwrap();
    let out = bin()
        .arg("--commit")
        .arg(head.trim())
        .current_dir(repo.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "{:?}", out.stderr);
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        stdout.contains("Author: Test User <test@example.com>"),
        "{stdout}"
    );
}

#[test]
fn commit_flag_from_subdirectory() {
    let repo = make_repo();
    let head = String::from_utf8(git(repo.path(), &["rev-parse", "HEAD"]).stdout).unwrap();
    let head = head.trim().to_string();
    std::fs::create_dir(repo.path().join("sub")).unwrap();
    let out = bin()
        .arg("--commit")
        .arg(&head)
        .current_dir(repo.path().join("sub"))
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        stdout.starts_with(&format!("commit {head}\n")),
        "should find the repo from a subdirectory:\n{stdout}"
    );
}

#[test]
fn commit_flag_selects_stash_commit() {
    let repo = make_repo();
    // A change in the working tree, then stash it: refs/stash becomes a ref,
    // so rev-list --all (the lookup universe) includes the stash commit.
    std::fs::write(repo.path().join("a.txt"), "bye\n").unwrap();
    git(repo.path(), &["stash", "-q"]);
    let stash = String::from_utf8(git(repo.path(), &["rev-parse", "refs/stash"]).stdout).unwrap();
    let stash = stash.trim().to_string();
    let out = bin()
        .arg("--commit")
        .arg(&stash)
        .current_dir(repo.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "{:?}", out.stderr);
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        stdout.starts_with(&format!("commit {stash}\n")),
        "a stash commit must be selectable:\n{stdout}"
    );
}

#[test]
fn commit_flag_works_in_bare_repo() {
    let repo = make_repo();
    let bare = repo.path().join("bare");
    git(
        repo.path(),
        &["clone", "-q", "--bare", ".", bare.to_str().unwrap()],
    );
    let head = String::from_utf8(git(&bare, &["rev-parse", "HEAD"]).stdout).unwrap();
    let head = head.trim();
    let out = bin()
        .arg("--commit")
        .arg(&head[..8])
        .current_dir(&bare)
        .output()
        .unwrap();
    assert!(out.status.success(), "{:?}", out.stderr);
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        stdout.starts_with(&format!("commit {head}\n")),
        "a bare repository must be treated as a repository:\n{stdout}"
    );
}

#[test]
fn config_author_does_not_override_real_commit() {
    let repo = make_repo();
    let cfg_path = repo.path().join("config.json");
    std::fs::write(
        &cfg_path,
        r#"{
    "commit": { "author_name": "Config User", "author_email": "config@x.com" }
}"#,
    )
    .unwrap();

    let out = bin()
        .arg("--config")
        .arg(&cfg_path)
        .current_dir(repo.path())
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        stdout.contains("Author: Test User <test@example.com>"),
        "config author must not mask the real commit author:\n{stdout}"
    );
    assert!(
        !stdout.contains("Config User"),
        "config author leaked into the output:\n{stdout}"
    );

    // An explicit --author still wins over the picked commit.
    let out = bin()
        .arg("--author")
        .arg("CLI User")
        .arg("--config")
        .arg(&cfg_path)
        .current_dir(repo.path())
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        stdout.contains("Author: CLI User <test@example.com>"),
        "--author must force the name:\n{stdout}"
    );
}

#[test]
fn text_line_renders_between_groups() {
    let dir = tempfile::tempdir().unwrap();
    let cfg_path = dir.path().join("config.json");
    std::fs::write(
        &cfg_path,
        r#"{
    "sections": {
        "first": { "name": "First", "email": "first@x.com" },
        "blank": "",
        "second": { "name": "Second", "email": "second@x.com" }
    },
    "order": ["first", "blank", "second"]
}"#,
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
    // The body is indented like git, so strip the indent when locating lines.
    let first = lines
        .iter()
        .position(|l| l.trim_start().starts_with("Co-Authored-By: First"))
        .unwrap();
    let second = lines
        .iter()
        .position(|l| l.trim_start().starts_with("Co-Authored-By: Second"))
        .unwrap();
    assert_eq!(
        second,
        first + 2,
        "empty text line between the two trailers"
    );
    assert_eq!(
        *lines.last().unwrap(),
        "    Co-Authored-By: Second <second@x.com>",
        "no trailer after the last one"
    );
}

#[cfg(unix)]
#[test]
fn text_section_with_fields_renders_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let cfg_path = dir.path().join("config.json");
    std::fs::write(
        &cfg_path,
        r#"{
    "sections": {
        "bot_text": {
            "text": "This commit is generated by Bot {bot_version}",
            "fields": { "bot_version": { "command": "printf 2.1.223" } }
        },
        "bot": {
            "name": "Bot {bot_version}",
            "email": "noreply@example.com",
            "fields": { "bot_version": { "command": "printf 2.1.223" } }
        }
    },
    "order": ["bot_text", "bot"]
}"#,
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
    let lines: Vec<&str> = stdout.lines().collect();
    let text = lines
        .iter()
        .position(|l| *l == "    This commit is generated by Bot 2.1.223")
        .expect("text line with resolved field must render:\n{stdout}");
    let trailer = lines
        .iter()
        .position(|l| l.starts_with("    Co-Authored-By: Bot 2.1.223"))
        .expect("bot trailer must render:\n{stdout}");
    assert_eq!(
        trailer,
        text + 1,
        "text line must appear right before the bot trailer"
    );
}

#[test]
fn order_omits_section_not_rendered() {
    let dir = tempfile::tempdir().unwrap();
    let cfg_path = dir.path().join("config.json");
    std::fs::write(
        &cfg_path,
        r#"{
    "sections": {
        "shown": { "name": "Shown", "email": "shown@x.com" },
        "hidden": { "name": "Hidden", "email": "hidden@x.com" }
    },
    "order": ["shown"]
}"#,
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
        stdout.contains("Co-Authored-By: Shown"),
        "listed section must render:\n{stdout}"
    );
    assert!(
        !stdout.contains("Hidden"),
        "section omitted from order must not render:\n{stdout}"
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
    // Point --config at a path that cannot exist so the developer's real
    // config (~/.config/blamefetch/config.json) cannot leak into the output:
    // load falls back to the embedded defaults with a warning on stderr.
    let missing = std::env::temp_dir().join(format!("blamefetch-no-config-{}", std::process::id()));
    let out = bin()
        .arg("--print-config")
        .arg("--config")
        .arg(&missing)
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        stdout.trim_start().starts_with('{'),
        "must print JSON:\n{stdout}"
    );
    assert!(stdout.contains("\"sections\""));
    assert!(stdout.contains("\"order\""));
    assert!(stdout.contains("\"os\""));
    assert!(
        !stdout.contains("sample"),
        "defaults must not contain sample fixtures"
    );
    // Section defaults live in the sources now; the embedded config only
    // declares the roster, so no name/email may leak into the output.
    assert!(
        !stdout.contains("@system.invalid"),
        "defaults must not carry email addresses:\n{stdout}"
    );
    assert!(
        !stdout.contains("\"name\""),
        "defaults must not carry section names:\n{stdout}"
    );
}

#[test]
fn example_config_is_subset_of_print_config() {
    // README says example/config.json is the config behind the sample
    // output. `--print-config` prints the *effective* config (built-in
    // probe sections merged in), so the file must be a subset of it —
    // every declared section survives unchanged, and the resolved order
    // is exactly the declared one.
    let example = Path::new(env!("CARGO_MANIFEST_DIR")).join("example/config.json");
    let out = bin()
        .arg("--print-config")
        .arg("--config")
        .arg(&example)
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    let printed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let file: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&example).unwrap()).unwrap();

    let printed_sections = printed["sections"].as_object().unwrap();
    let file_sections = file["sections"].as_object().unwrap();
    for (key, entry) in file_sections {
        assert_eq!(
            printed_sections.get(key),
            Some(entry),
            "section {key:?} declared in example/config.json must appear unchanged in --print-config output"
        );
    }
    assert_eq!(
        printed["order"], file["order"],
        "--print-config must materialize exactly the order declared in example/config.json"
    );

    // The built-in probe sections are compiled in, not declared.
    for kind in ["os", "kernel", "cpu", "gpu", "wm"] {
        assert!(
            printed_sections.contains_key(kind),
            "--print-config must include the built-in {kind} section even though example/config.json does not declare it"
        );
    }
}

#[cfg(unix)]
#[test]
fn print_config_does_not_run_commands() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("marker");
    let cfg_path = dir.path().join("config.json");
    std::fs::write(
        &cfg_path,
        format!(
            r#"{{
    "sections": {{
        "bot": {{
            "name": "Bot",
            "email": "bot@x.com",
            "fields": {{ "version": {{ "command": "touch {}" }} }}
        }}
    }}
}}"#,
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
    let cfg_path = dir.path().join("config.json");
    std::fs::write(
        &cfg_path,
        r#"{
    "sections": {
        "bots": {
            "name": "Bots {version}",
            "email": "bots@example.com",
            "fields": { "version": { "command": "false" } }
        }
    }
}"#,
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
        !stdout.contains("Bots"),
        "a failed command without fallback must suppress its trailer:\n{stdout}"
    );
    assert!(
        stdout.contains("Co-Authored-By:"),
        "other sections (the built-in roster) must still render:\n{stdout}"
    );
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("blamefetch: warning:"), "stderr: {stderr}");
}

/// Polls until no process matches the marker (SIGKILL delivery is async).
#[cfg(unix)]
fn wait_for_no_process(marker: &str) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        let found = Command::new("pgrep")
            .args(["-f", marker])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !found {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "process {marker:?} still alive 2s after the run"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

#[cfg(unix)]
#[test]
fn hanging_section_is_skipped_after_timeout() {
    let dir = tempfile::tempdir().unwrap();
    let cfg_path = dir.path().join("config.json");
    std::fs::write(
        &cfg_path,
        r#"{
    "sections": {
        "slow": {
            "name": "Slow {v}",
            "email": "slow@x.com",
            "fields": { "v": { "command": "sleep 301" } }
        },
        "fast": { "name": "Fast", "email": "fast@x.com" }
    }
}"#,
    )
    .unwrap();
    let start = std::time::Instant::now();
    let out = bin()
        .arg("--no-git")
        .arg("--config")
        .arg(&cfg_path)
        .env("BLAMEFETCH_SECTION_TIMEOUT_MS", "300")
        .current_dir(dir.path())
        .output()
        .unwrap();
    let elapsed = start.elapsed();
    assert!(out.status.success(), "a hung section must not fail the run");
    assert!(
        elapsed.as_secs() < 10,
        "run must not wait for the hung command (took {elapsed:?})"
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        stdout.contains("Co-Authored-By: Fast"),
        "fast section must still render:\n{stdout}"
    );
    assert!(
        !stdout.contains("Slow"),
        "hung section must be skipped:\n{stdout}"
    );
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("did not finish"), "stderr: {stderr}");
    // The command was cancelled, not failed: no second, misleading warning.
    assert!(
        !stderr.contains("command failed"),
        "cancelled command must not warn as a failure:\n{stderr}"
    );
    // The timed-out command must not outlive the run as an orphan.
    wait_for_no_process("^sleep 301$");
}

#[cfg(unix)]
#[test]
fn all_sections_hanging_still_exits() {
    let dir = tempfile::tempdir().unwrap();
    let cfg_path = dir.path().join("config.json");
    std::fs::write(
        &cfg_path,
        r#"{
    "sections": {
        "slow": {
            "name": "Slow {v}",
            "email": "slow@x.com",
            "fields": { "v": { "command": "sleep 302" } }
        }
    }
}"#,
    )
    .unwrap();
    let start = std::time::Instant::now();
    let out = bin()
        .arg("--no-git")
        .arg("--config")
        .arg(&cfg_path)
        .env("BLAMEFETCH_SECTION_TIMEOUT_MS", "300")
        .current_dir(dir.path())
        .output()
        .unwrap();
    let elapsed = start.elapsed();
    assert!(
        out.status.success(),
        "the run must exit successfully even when a section hangs"
    );
    assert!(
        elapsed.as_secs() < 10,
        "the run must still exit promptly (took {elapsed:?})"
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        !stdout.contains("Slow"),
        "hung section must be skipped:\n{stdout}"
    );
    // The timed-out command must not outlive the run as an orphan.
    wait_for_no_process("^sleep 302$");
}

#[cfg(unix)]
#[test]
fn sigint_cleans_up_running_section_command() {
    let dir = tempfile::tempdir().unwrap();
    let cfg_path = dir.path().join("config.json");
    let marker = dir.path().join("started");
    std::fs::write(
        &cfg_path,
        r#"{
    "sections": {
        "slow": {
            "name": "Slow {v}",
            "email": "slow@x.com",
            "fields": { "v": { "command": "touch started && sleep 303" } }
        }
    }
}"#,
    )
    .unwrap();
    let mut child = bin()
        .arg("--no-git")
        .arg("--config")
        .arg(&cfg_path)
        .env("BLAMEFETCH_SECTION_TIMEOUT_MS", "60000")
        .current_dir(dir.path())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    // Wait until the section command is actually running, then interrupt.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !marker.exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "section command did not start in time"
        );
        assert!(
            child.try_wait().unwrap().is_none(),
            "blamefetch exited before receiving SIGINT"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let kill = Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(kill.success(), "kill -INT failed");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "blamefetch did not exit after SIGINT"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    };
    assert_eq!(
        status.code(),
        Some(130),
        "expected the conventional SIGINT exit code (128+2), got {status:?}"
    );
    // The interrupted command must not outlive the run as an orphan.
    wait_for_no_process("^sleep 303$");
}

#[test]
fn utf8_config_renders_chinese_and_japanese() {
    let dir = tempfile::tempdir().unwrap();
    // Shared UTF-8 fixture: catgirl shi7ku9 content in Chinese and Japanese
    // ("shi7ku9" is a name and must stay untranslated).
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/utf8-shi7ku9.json");
    let out = bin()
        .arg("--no-git")
        .arg("--seed")
        .arg("42")
        .arg("--config")
        .arg(&fixture)
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        stdout.contains("Author: shi7ku9 <neko@shi7ku9.example>"),
        "Chinese author from config must render:\n{stdout}"
    );
    assert!(
        stdout.contains("    feat: 我是貓娘，搖著尾巴參上喵！"),
        "Chinese message from the config pool must render:\n{stdout}"
    );
    assert!(
        stdout.contains("    私は猫娘、お耳ぴこぴこで頑張るにゃ！"),
        "Japanese text line must render:\n{stdout}"
    );
    assert!(
        stdout.contains("    にゃん！私は猫娘、尻尾ふりふり參上だよ！"),
        "Japanese text-field line must render:\n{stdout}"
    );
    assert!(
        stdout.contains("    Co-Authored-By: shi7ku9 SSS <neko@shi7ku9.example>"),
        "Chinese co-author trailer with filled field must render:\n{stdout}"
    );
}

#[test]
fn utf8_config_print_config_preserves_chinese_and_japanese() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/utf8-shi7ku9.json");
    let out = bin()
        .arg("--print-config")
        .arg("--config")
        .arg(&fixture)
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        stdout.contains("我是貓娘"),
        "Chinese catgirl name must survive --print-config:\n{stdout}"
    );
    assert!(
        stdout.contains("私は猫娘、お耳ぴこぴこで頑張るにゃ！"),
        "Japanese text must survive --print-config:\n{stdout}"
    );

    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(parsed["commit"]["author_name"], "shi7ku9");
    assert_eq!(
        parsed["sections"]["メモ"],
        "私は猫娘、お耳ぴこぴこで頑張るにゃ！"
    );
    assert_eq!(parsed["sections"]["貓娘"]["name"], "shi7ku9 {rank}");
    assert_eq!(
        parsed["messages"]["pool"][0],
        "feat: 我是貓娘，搖著尾巴參上喵！"
    );
}

#[test]
fn color_flag_forces_ansi_when_piped() {
    let repo = make_repo();
    let minimal = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/minimal.json");
    // Clean color env: without the fix, `colored` suppresses ANSI for a
    // non-TTY stdout, so the explicit --color flag must still force it.
    let out = bin()
        .arg("--no-git")
        .arg("--seed")
        .arg("42")
        .arg("--config")
        .arg(&minimal)
        .arg("--color")
        .current_dir(repo.path())
        .env_remove("NO_COLOR")
        .env_remove("CLICOLOR")
        .env_remove("CLICOLOR_FORCE")
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(
        out.stdout.windows(2).any(|w| w == b"\x1b["),
        "--color must emit ANSI escapes even when stdout is piped"
    );
}

#[test]
fn color_flag_wins_over_no_color_env() {
    let repo = make_repo();
    let minimal = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/minimal.json");
    let out = bin()
        .arg("--no-git")
        .arg("--seed")
        .arg("42")
        .arg("--config")
        .arg(&minimal)
        .arg("--color")
        .current_dir(repo.path())
        .env("NO_COLOR", "1")
        .env_remove("CLICOLOR")
        .env_remove("CLICOLOR_FORCE")
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(
        out.stdout.windows(2).any(|w| w == b"\x1b["),
        "explicit --color must win over NO_COLOR"
    );
}

/// A crafted commit whose message contains raw terminal escape sequences must
/// not reach stdout: blamefetch renders commit data verbatim, so without
/// sanitization ESC bytes (title-set, clear-screen, color) are emitted.
#[test]
fn commit_message_escape_sequences_are_stripped() {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q"]);
    git(dir.path(), &["config", "user.name", "Test User"]);
    git(dir.path(), &["config", "user.email", "test@example.com"]);
    std::fs::write(dir.path().join("a.txt"), "hi\n").unwrap();
    git(dir.path(), &["add", "."]);
    let evil = "\x1b]0;EVIL-TITLE\x07\x1b[2J\x1b[31mRED ESCAPES\x1b[0m";
    git(
        dir.path(),
        &["-c", "commit.gpgsign=false", "commit", "-q", "-m", evil],
    );
    let hash = String::from_utf8(git(dir.path(), &["rev-parse", "HEAD"]).stdout).unwrap();
    let hash = hash.trim().to_string();
    let out = bin()
        .arg("--commit")
        .arg(&hash)
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(
        !out.stdout.contains(&b'\x1b'),
        "ESC bytes from a crafted commit message must be stripped from output"
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        stdout.contains("RED ESCAPES"),
        "visible message text must be preserved:\n{stdout}"
    );
}
