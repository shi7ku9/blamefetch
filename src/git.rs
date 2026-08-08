use rand::RngExt;
use rand::rngs::StdRng;
use rand::seq::IndexedRandom;

use crate::config::Config;

pub trait GitShell {
    fn output(&self, args: &[&str]) -> Option<String>;
}

pub struct RealGit;

impl GitShell for RealGit {
    fn output(&self, args: &[&str]) -> Option<String> {
        crate::util::cmd_output("git", args)
    }
}

#[derive(Debug, Clone)]
pub struct GitData {
    pub hash: String,
    pub message: String,
    pub author_name: Option<String>,
    pub author_email: Option<String>,
    /// Author date rendered like git's default `%ad` (e.g. "Thu Aug 6 05:32:10
    /// 2026 +0800"); `None` when unavailable, in which case the Date line is
    /// skipped.
    pub date: Option<String>,
}

impl GitData {
    pub fn random(config: &Config, rng: &mut StdRng) -> Self {
        Self {
            hash: random_hash(rng),
            message: random_message(config, rng),
            author_name: None,
            author_email: None,
            date: Some(random_date(rng)),
        }
    }
}

/// Why a `--commit <PREFIX>` lookup failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommitError {
    /// The prefix is empty or not hexadecimal, so it can never match.
    InvalidPrefix,
    /// No commit hash in the repository starts with the prefix.
    NoMatch,
    /// The repository could not be read (git failed), so the lookup cannot
    /// be completed.
    GitFailed,
    /// Exactly one commit matched, but its commit data could not be read.
    ReadFailed,
    /// More than one commit hash starts with the prefix; the candidates are
    /// listed for the user to disambiguate.
    Multiple(Vec<String>),
}

pub fn git_data(shell: &dyn GitShell, rng: &mut StdRng) -> Option<GitData> {
    if !is_in_repo(shell) {
        return None;
    }
    let hash = random_commit(shell, rng)?;
    commit_data(shell, &hash)
}

/// Loads the full commit record for one hash (git prints lowercase hex).
/// Fail-closed: any failed probe returns `None`; an empty message is valid.
fn commit_data(shell: &dyn GitShell, hash: &str) -> Option<GitData> {
    let message = commit_message(shell, hash)?;
    let author_name = commit_author_name(shell, hash)?;
    let author_email = commit_author_email(shell, hash)?;
    let date = commit_date(shell, hash)?;
    Some(GitData {
        hash: hash.to_string(),
        message,
        author_name: Some(author_name),
        author_email: Some(author_email),
        date: Some(date),
    })
}

/// Finds the commit whose hash starts with `prefix` (case-insensitive) among
/// every commit reachable from any ref (`rev-list --all`, the same universe
/// as the random picker). Exactly one match selects that commit; zero and
/// multiple matches are errors, as are a non-hex `prefix` and a failed
/// `rev-list` (the repository could not be read).
pub fn commit_by_prefix(shell: &dyn GitShell, prefix: &str) -> Result<GitData, CommitError> {
    let prefix = prefix.to_ascii_lowercase();
    if prefix.is_empty() || !prefix.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(CommitError::InvalidPrefix);
    }
    let all = shell
        .output(&["rev-list", "--all"])
        .ok_or(CommitError::GitFailed)?;
    let matches: Vec<String> = all
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_ascii_lowercase)
        .filter(|hash| hash.starts_with(&prefix))
        .collect();
    match matches.len() {
        0 => Err(CommitError::NoMatch),
        1 => commit_data(shell, &matches[0]).ok_or(CommitError::ReadFailed),
        _ => Err(CommitError::Multiple(matches)),
    }
}

/// True when the current directory is inside a Git repository (work tree or
/// bare). `rev-parse --git-dir` succeeds for both, unlike
/// `--is-inside-work-tree`, which reports `false` in a bare repository.
pub fn is_in_repo(shell: &dyn GitShell) -> bool {
    shell
        .output(&["rev-parse", "--git-dir"])
        .map(|s| !s.is_empty())
        .unwrap_or(false)
}

fn random_commit(shell: &dyn GitShell, rng: &mut StdRng) -> Option<String> {
    let all = shell.output(&["rev-list", "--all"])?;
    let hashes: Vec<&str> = all
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    hashes.choose(rng).map(|s| s.to_string())
}

fn commit_message(shell: &dyn GitShell, hash: &str) -> Option<String> {
    shell
        .output(&["log", "-1", "--format=%B", hash])
        .map(|s| s.trim_end().to_string())
}

/// Author of the selected commit itself (from the commit, not the current git config).
fn commit_author_name(shell: &dyn GitShell, hash: &str) -> Option<String> {
    shell
        .output(&["log", "-1", "--format=%an", hash])
        .filter(|s| !s.is_empty())
}

fn commit_author_email(shell: &dyn GitShell, hash: &str) -> Option<String> {
    shell
        .output(&["log", "-1", "--format=%ae", hash])
        .filter(|s| !s.is_empty())
}

/// Author date of the selected commit, in git's default human-readable format
/// (e.g. "Thu Aug 6 05:32:10 2026 +0800"). `--date=default` pins the format so
/// it matches `random_date` regardless of the user's `log.date` config.
fn commit_date(shell: &dyn GitShell, hash: &str) -> Option<String> {
    shell
        .output(&["log", "-1", "--date=default", "--format=%ad", hash])
        .filter(|s| !s.is_empty())
}

/// Random author date within the last year (never in the future), formatted
/// like git's default `%ad`. `%-d` renders the unpadded day, matching git's
/// `Thu Aug 6 05:32:10 2026 +0800` rather than a zero- or space-padded one.
fn random_date(rng: &mut StdRng) -> String {
    const SECONDS_PER_YEAR: u64 = 365 * 24 * 60 * 60;
    let back = rng.random_range(0..=SECONDS_PER_YEAR);
    let when = chrono::Local::now() - chrono::TimeDelta::seconds(back as i64);
    when.format("%a %b %-d %H:%M:%S %Y %z").to_string()
}

pub fn random_hash(rng: &mut StdRng) -> String {
    const HEX: &[u8] = b"0123456789abcdef";
    (0..40)
        .map(|_| HEX[rng.random_range(0..16)] as char)
        .collect()
}

pub fn random_message(config: &Config, rng: &mut StdRng) -> String {
    config
        .messages
        .pool
        .choose(rng)
        .cloned()
        .unwrap_or_default()
}

pub fn username() -> Option<String> {
    std::env::var("USER")
        .ok()
        .or_else(|| std::env::var("LOGNAME").ok())
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use rand::SeedableRng;
    use rand::rngs::StdRng;

    use std::collections::BTreeMap;

    use crate::config::{Config, MessagesConfig};

    use super::{
        CommitError, GitData, GitShell, commit_by_prefix, git_data, is_in_repo, random_hash,
        random_message,
    };

    #[derive(Default)]
    struct FakeGit {
        map: HashMap<String, Option<String>>,
    }

    impl FakeGit {
        fn set(&mut self, args: &[&str], out: Option<String>) {
            self.map.insert(args.join(" "), out);
        }
    }

    impl GitShell for FakeGit {
        fn output(&self, args: &[&str]) -> Option<String> {
            self.map.get(&args.join(" ")).cloned().unwrap_or(None)
        }
    }

    fn rng() -> StdRng {
        StdRng::seed_from_u64(42)
    }

    #[test]
    fn is_in_repo_true() {
        let mut g = FakeGit::default();
        g.set(&["rev-parse", "--git-dir"], Some(".git".to_string()));
        assert!(is_in_repo(&g));
    }

    #[test]
    fn is_in_repo_false_when_missing() {
        let g = FakeGit::default();
        assert!(!is_in_repo(&g));
    }

    #[test]
    fn random_hash_is_40_hex() {
        let mut r = rng();
        let h = random_hash(&mut r);
        assert_eq!(h.len(), 40);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn random_message_from_pool() {
        let mut r = rng();
        let config = Config {
            commit: Default::default(),
            messages: MessagesConfig {
                pool: vec!["one".into(), "two".into()],
            },
            sections: BTreeMap::new(),
            order: None,
        };
        assert!(["one", "two"].contains(&random_message(&config, &mut r).as_str()));
    }

    #[test]
    fn git_data_in_repo() {
        let mut g = FakeGit::default();
        g.set(&["rev-parse", "--git-dir"], Some(".git".to_string()));
        g.set(&["rev-list", "--all"], Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\nbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string()));
        // Respond for both candidate hashes so the test holds regardless of which
        // seed 42 picks (commit author comes from the commit itself, not git config).
        for hash in [
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        ] {
            g.set(
                &["log", "-1", "--format=%B", hash],
                Some("feat: test\n".to_string()),
            );
            g.set(
                &["log", "-1", "--format=%an", hash],
                Some("Ann".to_string()),
            );
            g.set(
                &["log", "-1", "--format=%ae", hash],
                Some("ann@x.com".to_string()),
            );
            g.set(
                &["log", "-1", "--date=default", "--format=%ad", hash],
                Some("Thu Aug 6 05:32:10 2026 +0800".to_string()),
            );
        }
        let data = git_data(&g, &mut rng()).unwrap();
        assert!(
            data.hash == "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                || data.hash == "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
        );
        assert_eq!(data.message, "feat: test");
        assert_eq!(data.author_name.as_deref(), Some("Ann"));
        assert_eq!(data.author_email.as_deref(), Some("ann@x.com"));
        assert_eq!(data.date.as_deref(), Some("Thu Aug 6 05:32:10 2026 +0800"));
    }

    #[test]
    fn git_data_none_outside_repo() {
        let g = FakeGit::default();
        assert!(git_data(&g, &mut rng()).is_none());
    }

    /// A fake repo whose `rev-list --all` returns `hashes`, with full commit
    /// data (message/author/date) configured for every hash.
    fn fake_with_hashes(hashes: &[&str]) -> FakeGit {
        let mut g = FakeGit::default();
        g.set(&["rev-list", "--all"], Some(hashes.join("\n")));
        for hash in hashes {
            g.set(
                &["log", "-1", "--format=%B", hash],
                Some("feat: test\n".to_string()),
            );
            g.set(
                &["log", "-1", "--format=%an", hash],
                Some("Ann".to_string()),
            );
            g.set(
                &["log", "-1", "--format=%ae", hash],
                Some("ann@x.com".to_string()),
            );
            g.set(
                &["log", "-1", "--date=default", "--format=%ad", hash],
                Some("Thu Aug 6 05:32:10 2026 +0800".to_string()),
            );
        }
        g
    }

    #[test]
    fn commit_by_prefix_single_match_full_and_short() {
        let a = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let b = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let g = fake_with_hashes(&[a, b]);
        for prefix in ["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "aaaa", "a"] {
            let data = commit_by_prefix(&g, prefix).unwrap();
            assert_eq!(data.hash, a);
            assert_eq!(data.message, "feat: test");
            assert_eq!(data.author_name.as_deref(), Some("Ann"));
            assert_eq!(data.author_email.as_deref(), Some("ann@x.com"));
            assert_eq!(data.date.as_deref(), Some("Thu Aug 6 05:32:10 2026 +0800"));
        }
    }

    #[test]
    fn commit_by_prefix_is_case_insensitive() {
        let a = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let g = fake_with_hashes(&[a]);
        assert_eq!(commit_by_prefix(&g, "AAAA").unwrap().hash, a);
    }

    #[test]
    fn commit_by_prefix_multiple_matches_lists_all() {
        let a = "aaaa11111111111111111111111111111111111111";
        let b = "aaaa22222222222222222222222222222222222222";
        let g = fake_with_hashes(&[a, b]);
        let err = commit_by_prefix(&g, "aaaa").unwrap_err();
        assert_eq!(
            err,
            CommitError::Multiple(vec![a.to_string(), b.to_string()])
        );
    }

    #[test]
    fn commit_by_prefix_no_match() {
        let g = fake_with_hashes(&["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"]);
        assert_eq!(
            commit_by_prefix(&g, "bbbb").unwrap_err(),
            CommitError::NoMatch
        );
    }

    #[test]
    fn commit_by_prefix_rejects_empty_and_non_hex() {
        let g = fake_with_hashes(&["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"]);
        assert_eq!(
            commit_by_prefix(&g, "").unwrap_err(),
            CommitError::InvalidPrefix
        );
        assert_eq!(
            commit_by_prefix(&g, "xyz").unwrap_err(),
            CommitError::InvalidPrefix
        );
        assert_eq!(
            commit_by_prefix(&g, "你").unwrap_err(),
            CommitError::InvalidPrefix
        );
    }

    #[test]
    fn commit_by_prefix_missing_rev_list_is_git_failed() {
        let g = FakeGit::default();
        assert_eq!(
            commit_by_prefix(&g, "aaaa").unwrap_err(),
            CommitError::GitFailed
        );
    }

    #[test]
    fn commit_by_prefix_read_failure_is_read_failed() {
        let mut g = FakeGit::default();
        let a = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        // rev-list returns a single hash, but no `log` entries are configured,
        // so reading the commit data fails.
        g.set(&["rev-list", "--all"], Some(a.to_string()));
        assert_eq!(
            commit_by_prefix(&g, "aaaa").unwrap_err(),
            CommitError::ReadFailed
        );
    }

    #[test]
    fn git_data_random_mode() {
        let config = Config {
            commit: Default::default(),
            messages: MessagesConfig {
                pool: vec!["hi".into()],
            },
            sections: BTreeMap::new(),
            order: None,
        };
        let data = GitData::random(&config, &mut rng());
        assert_eq!(data.message, "hi");
        assert_eq!(data.hash.len(), 40);
        assert!(data.author_name.is_none());
        assert_random_date(&data);
    }

    /// A random date must be present, shaped like git's default `%ad`
    /// ("Thu Aug 6 05:32:10 2026 +0800": weekday, month, unpadded day, time,
    /// year, offset), within the last year, and never in the future.
    fn assert_random_date(data: &GitData) {
        let date = data.date.as_deref().expect("random mode has a date");
        let dt = chrono::DateTime::parse_from_str(date, "%a %b %d %H:%M:%S %Y %z")
            .unwrap_or_else(|e| panic!("date not git-formatted ({e}): {date:?}"));
        let now = chrono::Utc::now();
        let dt = dt.with_timezone(&chrono::Utc);
        assert!(dt <= now, "random date must not be in the future: {date:?}");
        assert!(
            dt >= now - chrono::TimeDelta::days(366),
            "random date must be within a year: {date:?}"
        );
    }
}
