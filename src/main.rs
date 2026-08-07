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

    let ctx = SourceContext::new();
    let mut cache = sources::CommandCache::new();
    let mut body_items = Vec::new();
    for (key, entry) in config.ordered_sections() {
        match entry {
            config::SectionEntry::TextLine(line) => {
                body_items.push(render::BodyItem::Text(line.clone()));
            }
            config::SectionEntry::TextField(tf) => {
                if let Some(text) = sources::render_text(tf, &mut cache) {
                    body_items.push(render::BodyItem::Text(text));
                }
            }
            config::SectionEntry::CoAuthor(cfg) => {
                if let Some(co_author) = sources::render_co_author(cfg, &key, &ctx, &mut cache) {
                    body_items.push(render::BodyItem::Trailer(co_author));
                }
            }
        }
    }

    let opts = render::RenderOptions { color: cli.color };
    print!("{}", render::render_commit(&git_data, &body_items, &opts));
    ExitCode::SUCCESS
}
