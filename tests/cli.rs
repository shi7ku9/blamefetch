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
fn example_config_print_config_matches_file() {
    // README claims example/config.json is the exact JSON produced by
    // `blamefetch --config example/config.json --print-config`; pin it.
    let example = Path::new(env!("CARGO_MANIFEST_DIR")).join("example/config.json");
    let out = bin()
        .arg("--print-config")
        .arg("--config")
        .arg(&example)
        .output()
        .unwrap();
    assert!(out.status.success());
    let file = std::fs::read_to_string(&example).unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert_eq!(
        stdout, file,
        "example/config.json must match --print-config output"
    );
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
            "fields": { "v": { "command": "sleep 30" } }
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
            "fields": { "v": { "command": "sleep 30" } }
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
    // The orphaned `sleep 30` child lingers up to 30 s — harmless, and the
    // process itself has already exited.
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        !stdout.contains("Slow"),
        "hung section must be skipped:\n{stdout}"
    );
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
        stdout.contains("Author: 貓娘 shi7ku9 <neko@shi7ku9.example>"),
        "Chinese author from config must render:\n{stdout}"
    );
    assert!(
        stdout.contains("    feat: 貓娘 shi7ku9 參上！"),
        "Chinese message from the config pool must render:\n{stdout}"
    );
    assert!(
        stdout.contains("    今日も猫娘 shi7ku9 と一緒に頑張るにゃ！"),
        "Japanese text line must render:\n{stdout}"
    );
    assert!(
        stdout.contains("    猫娘 shi7ku9 参上！にゃん"),
        "Japanese text-field line must render:\n{stdout}"
    );
    assert!(
        stdout.contains("    Co-Authored-By: 貓娘 shi7ku9 SSS <neko@shi7ku9.example>"),
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
        stdout.contains("貓娘 shi7ku9"),
        "Chinese catgirl name must survive --print-config:\n{stdout}"
    );
    assert!(
        stdout.contains("今日も猫娘 shi7ku9 と一緒に頑張るにゃ！"),
        "Japanese text must survive --print-config:\n{stdout}"
    );

    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(parsed["commit"]["author_name"], "貓娘 shi7ku9");
    assert_eq!(
        parsed["sections"]["メモ"],
        "今日も猫娘 shi7ku9 と一緒に頑張るにゃ！"
    );
    assert_eq!(parsed["sections"]["貓娘"]["name"], "貓娘 shi7ku9 {rank}");
    assert_eq!(parsed["messages"]["pool"][0], "feat: 貓娘 shi7ku9 參上！");
}
