mod cli;
mod config;
mod git;
mod render;
mod sources;
mod template;
mod util;

use std::collections::HashMap;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use clap::Parser;
use rand::SeedableRng;
use rand::rngs::StdRng;

use cli::Cli;
use render::BodyItem;
use sources::SourceContext;

/// Default budget per section before it is skipped; overridable via the
/// `BLAMEFETCH_SECTION_TIMEOUT_MS` environment variable.
const DEFAULT_SECTION_TIMEOUT_MS: u64 = 5000;

fn section_timeout_ms(raw: Option<&str>) -> u64 {
    match raw.and_then(|s| s.parse::<u64>().ok()) {
        Some(v) if v > 0 => v,
        _ => DEFAULT_SECTION_TIMEOUT_MS,
    }
}

/// First 20 candidate hashes, each indented, plus a truncation note when the
/// list is longer — a short prefix can match thousands of commits.
fn candidate_lines(hashes: &[String]) -> Vec<String> {
    const MAX_LISTED: usize = 20;
    let mut lines: Vec<String> = hashes
        .iter()
        .take(MAX_LISTED)
        .map(|hash| format!("  {hash}"))
        .collect();
    if hashes.len() > MAX_LISTED {
        lines.push(format!("  … and {} more", hashes.len() - MAX_LISTED));
    }
    lines
}

/// Resolves all sections concurrently — one detached thread per TextField /
/// CoAuthor section — and returns the body items in `sections` order. A
/// section that does not finish within `timeout` is skipped with a warning.
/// Workers are deliberately never joined: a thread stuck in a blocking
/// syscall (e.g. statfs on a hung NFS mount) must not hold up the run —
/// process exit terminates it, and a late worker's send is dropped with the
/// receiver.
fn resolve_body(
    sections: &[(String, &config::SectionEntry)],
    ctx: &Arc<SourceContext>,
    cache: &Arc<sources::CommandCache>,
    timeout: Duration,
) -> Vec<BodyItem> {
    let (tx, rx) = mpsc::channel::<(usize, Option<BodyItem>)>();
    let mut results: HashMap<usize, BodyItem> = HashMap::new();
    let mut pending: HashMap<usize, Instant> = HashMap::new();

    for (idx, (key, entry)) in sections.iter().enumerate() {
        match entry {
            config::SectionEntry::TextLine(line) => {
                // Instant; no thread needed.
                results.insert(idx, BodyItem::Text(line.clone()));
            }
            _ => {
                let tx = tx.clone();
                let ctx = Arc::clone(ctx);
                let cache = Arc::clone(cache);
                let key = key.clone();
                // `entry` is a `&&SectionEntry` here; `.clone()` would copy the
                // reference (`Clone for &T` wins the probe), so deref first.
                let entry = (*entry).clone();
                thread::spawn(move || {
                    let item = match &entry {
                        config::SectionEntry::TextField(tf) => {
                            sources::render_text(tf, cache.as_ref()).map(BodyItem::Text)
                        }
                        config::SectionEntry::CoAuthor(cfg) => {
                            sources::render_co_author(cfg, &key, ctx.as_ref(), cache.as_ref())
                                .map(BodyItem::Trailer)
                        }
                        config::SectionEntry::TextLine(_) => unreachable!(),
                    };
                    let _ = tx.send((idx, item)); // ignored once main gave up
                });
                pending.insert(idx, Instant::now() + timeout);
            }
        }
    }
    drop(tx); // main holds no sender; Disconnected means all workers exited

    while !pending.is_empty() {
        let now = Instant::now();
        // Give up on expired sections first.
        let expired: Vec<usize> = pending
            .iter()
            .filter(|(_, deadline)| **deadline <= now)
            .map(|(idx, _)| *idx)
            .collect();
        for idx in expired {
            let (key, _) = &sections[idx];
            eprintln!(
                "blamefetch: warning: section {key:?} did not finish within {} ms; skipping",
                timeout.as_millis()
            );
            pending.remove(&idx);
        }
        if pending.is_empty() {
            break;
        }
        let wait = pending
            .values()
            .map(|deadline| deadline.saturating_duration_since(now))
            .min()
            .unwrap();
        match rx.recv_timeout(wait) {
            Ok((idx, item)) => {
                if pending.remove(&idx).is_some()
                    && let Some(item) = item
                {
                    results.insert(idx, item);
                }
            }
            Err(RecvTimeoutError::Timeout) => {} // loop re-drains expired
            Err(RecvTimeoutError::Disconnected) => {
                // Every remaining worker exited without sending a result
                // (typically a panic). Do not clear them silently.
                for idx in pending.keys().copied() {
                    let (key, _) = &sections[idx];
                    eprintln!(
                        "blamefetch: warning: section {key:?} exited without a result; skipping"
                    );
                }
                pending.clear();
            }
        }
    }

    sections
        .iter()
        .enumerate()
        .filter_map(|(idx, _)| results.remove(&idx))
        .collect()
}

fn main() -> ExitCode {
    util::install_interrupt_cleanup();
    let cli = Cli::parse();
    let config = config::Config::load(cli.config.as_deref());

    if cli.print_config {
        println!("{}", config.to_json());
        return ExitCode::SUCCESS;
    }
    if cli.list_sources {
        for source in sources::all_sources() {
            println!("{}", source.kind());
        }
        return ExitCode::SUCCESS;
    }

    let mut rng: StdRng = match cli.seed {
        Some(seed) => StdRng::seed_from_u64(seed),
        None => rand::make_rng(),
    };

    let (git, source_note) = if let Some(prefix) = cli.commit.as_deref() {
        if !git::is_in_repo(&git::RealGit) {
            eprintln!("blamefetch: error: --commit requires being inside a Git repository");
            return ExitCode::FAILURE;
        }
        match git::commit_by_prefix(&git::RealGit, prefix) {
            Ok(data) => (data, None),
            Err(git::CommitError::Multiple(hashes)) => {
                eprintln!(
                    "blamefetch: error: --commit '{prefix}' matches {} commits:",
                    hashes.len()
                );
                for line in candidate_lines(&hashes) {
                    eprintln!("{line}");
                }
                return ExitCode::FAILURE;
            }
            Err(git::CommitError::NoMatch) => {
                eprintln!(
                    "blamefetch: error: --commit '{prefix}' does not match any commit in this repository (must be a non-empty hexadecimal hash prefix)"
                );
                return ExitCode::FAILURE;
            }
            Err(git::CommitError::ReadFailed) => {
                eprintln!(
                    "blamefetch: error: --commit '{prefix}' matched a commit but its data could not be read"
                );
                return ExitCode::FAILURE;
            }
        }
    } else if cli.no_git {
        (git::GitData::random(&config, &mut rng), Some("--no-git"))
    } else {
        match git::git_data(&git::RealGit, &mut rng) {
            Some(data) => (data, None),
            None => (
                git::GitData::random(&config, &mut rng),
                Some("not inside a git repository"),
            ),
        }
    };
    if cli.verbose > 0
        && let Some(note) = source_note
    {
        eprintln!("blamefetch: using random commit data ({note})");
    }

    let fallback_user = git::username().unwrap_or_else(|| "unknown".to_string());
    // Precedence: explicit CLI flags, then the picked commit's own author, then
    // the config default (only used outside a repo / --no-git), then $USER.
    let author_name = cli
        .author
        .or_else(|| git.author_name.clone())
        .or_else(|| config.commit.author_name.clone())
        .unwrap_or_else(|| fallback_user.clone());
    let author_email = cli
        .email
        .or_else(|| git.author_email.clone())
        .or_else(|| config.commit.author_email.clone())
        .unwrap_or_else(|| format!("{fallback_user}@localhost"));

    let git_data = git::GitData {
        hash: cli.hash.unwrap_or(git.hash),
        message: cli.message.unwrap_or(git.message),
        author_name: Some(author_name),
        author_email: Some(author_email),
        date: git.date,
    };

    let timeout = Duration::from_millis(section_timeout_ms(
        std::env::var("BLAMEFETCH_SECTION_TIMEOUT_MS")
            .ok()
            .as_deref(),
    ));
    let ctx = Arc::new(SourceContext::new());
    let cache = Arc::new(sources::CommandCache::new());
    let sections: Vec<(String, &config::SectionEntry)> = config.ordered_sections();
    let body_items = resolve_body(&sections, &ctx, &cache, timeout);
    // A section that timed out leaves its command running; kill it so it does
    // not outlive this process as an orphan.
    util::kill_running_children();

    let opts = render::RenderOptions { color: cli.color };
    print!("{}", render::render_commit(&git_data, &body_items, &opts));
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn shared_state_is_send_sync() {
        // The parallel resolver shares these across threads; sysinfo does not
        // promise Send+Sync on System, so pin the property here.
        assert_send_sync::<SourceContext>();
        assert_send_sync::<sources::CommandCache>();
        assert_send_sync::<BodyItem>();
    }

    #[test]
    fn section_timeout_default_when_unset() {
        assert_eq!(section_timeout_ms(None), DEFAULT_SECTION_TIMEOUT_MS);
    }

    #[test]
    fn section_timeout_parses() {
        assert_eq!(section_timeout_ms(Some("300")), 300);
        assert_eq!(section_timeout_ms(Some("10000")), 10_000);
    }

    #[test]
    fn section_timeout_invalid_or_zero_falls_back() {
        assert_eq!(section_timeout_ms(Some("abc")), DEFAULT_SECTION_TIMEOUT_MS);
        assert_eq!(section_timeout_ms(Some("0")), DEFAULT_SECTION_TIMEOUT_MS);
        assert_eq!(section_timeout_ms(Some("-5")), DEFAULT_SECTION_TIMEOUT_MS);
    }

    #[test]
    fn candidate_lines_lists_all_under_limit() {
        let hashes = vec!["a".to_string(), "b".to_string()];
        assert_eq!(candidate_lines(&hashes), vec!["  a", "  b"]);
    }

    #[test]
    fn candidate_lines_truncates_after_twenty() {
        let hashes: Vec<String> = (0..25).map(|i| format!("{i:040x}")).collect();
        let lines = candidate_lines(&hashes);
        assert_eq!(lines.len(), 21);
        assert!(lines[20].ends_with("and 5 more"), "{:?}", lines[20]);
        assert_eq!(lines[19], format!("  {}", hashes[19]));
    }
}
