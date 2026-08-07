# blamefetch

A satirical neofetch that blames your whole machine for the commit.

`blamefetch` prints a fake `git show`-style commit — using a real commit from
the current repository when possible, or generated random data when not — and
credits your OS, kernel, host, user, shell, editor, and hardware as
`Co-Authored-By` trailers.

It never creates or modifies commits. It just prints.

## Features

- Picks a random commit from the current Git repository, or generates a random
  hash and message outside one (`--no-git` for pure randomness).
- Collects system information (OS, kernel, host, hostname, user, shell,
  terminal, window manager, uptime, CPU, GPU, memory, disk, locale) and renders
  each as a `Co-Authored-By` trailer.
- Fully configurable through `~/.config/blamefetch/config.json`, including
  custom co-authors, plain text lines, message pools, and shell-command fields.
- Reproducible output with `--seed` (except the `Date:` line, which is
  relative to the current time).
- Optional colored output with `--color`.

## Requirements

- Rust 1.95 or newer (see `rust-toolchain.toml`).
- `git` on `PATH` when you want real commit data (optional otherwise).
- Nix users can use the provided flake instead.

## Installation

Build and install from source:

```sh
cargo install --path .
```

Or run directly:

```sh
cargo run --release -- --no-git
```

With Nix:

```sh
nix run .#
```

For a development shell:

```sh
nix develop
```

## Usage

Run `blamefetch` inside a Git repository to use a real commit:

```sh
blamefetch
```

Outside a repository, or with `--no-git`, random commit data is generated.

| Flag | Description |
| --- | --- |
| `--no-git` | Skip reading the current Git repo; use pure random data. |
| `--hash <HASH>` | Force the commit hash. |
| `--message <MESSAGE>` | Force the commit message. |
| `--author <NAME>` | Force the author name. |
| `--email <EMAIL>` | Force the author email. |
| `--color` | Colorize the output. |
| `--seed <SEED>` | Seed the RNG for reproducible output. |
| `--list-sources` | List available co-author kinds and exit. |
| `--print-config` | Print the effective config (defaults merged with the config file) and exit. |
| `--config <PATH>` | Path to a config file. |
| `-v`, `--verbose` | Verbose output (repeatable). |

## Example output

Output varies by machine and configuration. A sample run:

```text
commit 2838daf6f0965b2da270de98d6b2c8d3a02870aa
Author: shiziku <shiziku@localhost>
Date:   Thu Aug 6 05:32:10 2026 +0800

    fix: blame the cache

    Co-Authored-By: NixOS 26.11.0 <os@system.invalid>
    Co-Authored-By: Linux 6.18.38 <kernel@system.invalid>
    Co-Authored-By: Aspire AGM16-71P <host@system.invalid>
    Co-Authored-By: shiziku-laptop <shiziku-laptop@host.local>
    Co-Authored-By: shiziku <shiziku@users.local>
    Co-Authored-By: zsh 5.9.1 <shell@system.invalid>
    Co-Authored-By: xterm-kitty <terminal@system.invalid>
    Co-Authored-By: Hyprland <wm@system.invalid>
    Co-Authored-By: 7 days, 2 hours, 43 mins <uptime@system.invalid>
    Co-Authored-By: Intel(R) Core(TM) Ultra 5 225H (14 threads) @ 1.80 GHz <cpu@system.invalid>
    Co-Authored-By: Intel Corporation Arrow Lake-P [Arc Pro 130T/140T] [8086:7d51] (rev 03) <gpu@system.invalid>
    Co-Authored-By: 4.00 GiB / 30.88 GiB <memory@system.invalid>
    Co-Authored-By: 278.15 GiB / 467.40 GiB <disk@system.invalid>
    Co-Authored-By: en_US.UTF-8 <locale@system.invalid>
    Co-Authored-By: Nvim v0.12.4 <editor@user.invalid>
    Co-Authored-By: blamefetch 0.1.0 <self@blamefetch.invalid>

    Co-Authored-By: Claude 2.1.222 <noreply@anthropic.com>
```

## Configuration

blamefetch embeds a default configuration and merges
`~/.config/blamefetch/config.json` on top of it (the exact path follows your
platform's config directory). The top-level keys are:

- `commit` — fallback `author_name` / `author_email`, used only when no real
  repo commit is picked (outside a git repository, or with `--no-git`); a real
  commit's own author wins. `--author` / `--email` always force.
- `messages` — the `pool` used for random commit messages.
- `sections` — one entry per output line: a co-author (object with
  `name`/`email`/`fields`), or a plain text line (string value; an object with
  a `text` key also supports `{placeholder}` fields).
- `order` — the exact display list: only listed sections render, in listed
  order; sections omitted from `order` are not shown.

`name` and `email` support `{placeholder}` templates filled by `fields`; field
values can be plain strings or shell commands:

```json
{
  "sections": {
    "nvim": {
      "name": "Nvim {nvim_version}",
      "email": "editor@user.invalid",
      "fields": {
        "nvim_version": { "command": "nvim --version | cut -d' ' -f2", "fallback": "" }
      }
    }
  }
}
```

blamefetch credits the machine by default and attributes no one else; anything
else it prints is your configuration.

See [Configuring blamefetch](docs/config.md) for a full tutorial.
`--print-config` shows the merged effective config, and `--list-sources` lists
the built-in kinds: `os`, `kernel`, `host`, `hostname`, `user`, `shell`,
`terminal`, `wm`, `uptime`, `cpu`, `gpu`, `memory`, `disk`, `locale`.

## Security and liability

blamefetch is display-only. It never creates, amends, pushes, or otherwise
modifies commits or Git history. When inside a repository, it only runs
read-only Git commands (`rev-parse`, `rev-list`, `log`).

blamefetch does not execute shell commands on its own. Commands run only if you
configure them in a `sections` entry via `name`, `email`, `fields`, or a text
section's `fields` with `{ "command": ... }`. Such commands run through your
platform shell (`sh -c` on
Unix, `cmd /C` on Windows) with the same permissions as the user who started
blamefetch, without a sandbox.

Only run config files you trust. You are responsible for reviewing any
configuration — including any commands inside it — before running blamefetch.
The project authors assume no liability for effects caused by user-supplied
commands or configuration.

## Development

```sh
cargo build
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

With Nix, the same checks are available as `nix flake check`.

## License

MIT — see [LICENSE](LICENSE).
