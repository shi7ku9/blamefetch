use std::path::PathBuf;

use clap::{ArgAction, Parser};

#[derive(Debug, Parser)]
#[command(
    name = "blamefetch",
    version,
    about = "A satirical neofetch that blames your whole machine for the commit"
)]
pub struct Cli {
    /// Skip reading the current git repo; use pure random data.
    #[arg(long)]
    pub no_git: bool,

    /// Force the commit hash.
    #[arg(long)]
    pub hash: Option<String>,

    /// Select the commit whose hash starts with this hex prefix
    /// (case-insensitive); errors when zero or multiple commits match.
    /// Conflicts with --no-git and --hash.
    #[arg(long, value_name = "PREFIX", conflicts_with_all = ["no_git", "hash"])]
    pub commit: Option<String>,

    /// Force the commit message.
    #[arg(long)]
    pub message: Option<String>,

    /// Force the author name.
    #[arg(long)]
    pub author: Option<String>,

    /// Force the author email.
    #[arg(long)]
    pub email: Option<String>,

    /// Force colored output, even when stdout is not a terminal or NO_COLOR is set.
    #[arg(long)]
    pub color: bool,

    /// Seed the RNG used for commit selection; the rest of the output still varies.
    #[arg(long)]
    pub seed: Option<u64>,

    /// List available co-author kinds and exit.
    #[arg(long)]
    pub list_sources: bool,

    /// Print the effective config (defaults merged with the config file) and exit.
    #[arg(long)]
    pub print_config: bool,

    /// Path to a config file.
    #[arg(long, value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// Verbose output.
    #[arg(short, long, action = ArgAction::Count)]
    pub verbose: u8,
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::Cli;

    #[test]
    fn parses_no_args() {
        let cli = Cli::try_parse_from(["blamefetch"]).unwrap();
        assert!(!cli.no_git);
        assert!(!cli.color);
        assert!(cli.hash.is_none());
        assert!(cli.seed.is_none());
    }

    #[test]
    fn parses_flags() {
        let cli = Cli::try_parse_from([
            "blamefetch",
            "--commit",
            "abc123",
            "--message",
            "hello",
            "--author",
            "Ann",
            "--email",
            "a@x.com",
            "--color",
            "--seed",
            "42",
            "--list-sources",
            "--print-config",
            "--config",
            "/tmp/cfg.toml",
            "-v",
        ])
        .unwrap();
        assert!(cli.color);
        assert_eq!(cli.commit.as_deref(), Some("abc123"));
        assert_eq!(cli.message.as_deref(), Some("hello"));
        assert_eq!(cli.author.as_deref(), Some("Ann"));
        assert_eq!(cli.email.as_deref(), Some("a@x.com"));
        assert_eq!(cli.seed, Some(42));
        assert!(cli.list_sources);
        assert!(cli.print_config);
        assert_eq!(
            cli.config.as_deref(),
            Some(std::path::Path::new("/tmp/cfg.toml"))
        );
        assert_eq!(cli.verbose, 1);
    }

    #[test]
    fn parses_no_git_and_hash() {
        let cli = Cli::try_parse_from(["blamefetch", "--no-git", "--hash", "abc123"]).unwrap();
        assert!(cli.no_git);
        assert_eq!(cli.hash.as_deref(), Some("abc123"));
        assert!(cli.commit.is_none());
    }
}
