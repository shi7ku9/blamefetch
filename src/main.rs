mod cli;
mod config;
mod git;
mod render;
mod sources;
mod template;
mod util;

use clap::Parser;
use rand::SeedableRng;
use rand::rngs::StdRng;
use std::process::ExitCode;

use cli::Cli;
use sources::SourceContext;

fn main() -> ExitCode {
    let cli = Cli::parse();
    let config = config::Config::load(cli.config.as_deref());

    if cli.print_config {
        println!("{}", config.to_toml());
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

    let (git, source_note) = if cli.no_git {
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
    let author_name = cli
        .author
        .or_else(|| config.commit.author_name.clone())
        .or_else(|| git.author_name.clone())
        .unwrap_or_else(|| fallback_user.clone());
    let author_email = cli
        .email
        .or_else(|| config.commit.author_email.clone())
        .or_else(|| git.author_email.clone())
        .unwrap_or_else(|| format!("{fallback_user}@localhost"));

    let git_data = git::GitData {
        hash: cli.hash.unwrap_or(git.hash),
        message: cli.message.unwrap_or(git.message),
        author_name: Some(author_name),
        author_email: Some(author_email),
    };

    let ctx = SourceContext::new();
    let mut cache = sources::CommandCache::new();
    let mut co_authors = Vec::new();
    for cfg in &config.co_authors {
        if let Some(co_author) = sources::render_co_author(cfg, &ctx, &mut cache) {
            co_authors.push(co_author);
        }
    }

    let opts = render::RenderOptions { color: cli.color };
    print!("{}", render::render_commit(&git_data, &co_authors, &opts));
    ExitCode::SUCCESS
}
