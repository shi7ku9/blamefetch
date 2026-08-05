# Configuring blamefetch

blamefetch embeds a default configuration and merges
`~/.config/blamefetch/config.toml` on top of it (the exact path follows your
platform's config directory). Run `blamefetch --print-config` to see the merged
result, and `blamefetch --list-sources` to list the built-in co-author kinds.

This guide focuses on the `[[co_authors]]` section. For installation and usage,
see the [README](../README.md).

## Anatomy of a co-author

Each `[[co_authors]]` entry renders one `Co-Authored-By:` trailer:

- `kind` — a built-in source such as `os` or `gpu`; omit it for a config-only
  entry.
- `enabled` — default `true`; set to `false` to disable an entry.
- `name` / `email` — a plain string with `{placeholder}` templates, or a
  `{ command = "...", fallback = "..." }` value.
- `fields` — a table that fills placeholders; each value is a plain string or a
  command.
- `blank_line_before` — insert a blank line before this trailer.

Rules to remember:

- Config `fields` override values from a built-in source.
- A command that fails without a `fallback` skips the whole trailer
  (fail-closed); with a `fallback`, the fallback is used instead.
- An empty resulting `name` or `email` skips the trailer.
- An unknown `kind` is silently skipped.
- Commands run through `sh -c` on Unix and `cmd /C` on Windows, are cached once
  per invocation, and have their output trimmed.

## Example: Claude

```toml
[[co_authors]]
blank_line_before=true
name = "Claude {version}"
email = "noreply@anthropic.com"

[co_authors.fields]
version = { command = "claude --version | cut -d' ' -f1", fallback = ""}
```

On a machine where `claude --version` prints `2.1.220 (Claude Code)`, the
command extracts `2.1.220` and renders:

```text
Co-Authored-By: Claude 2.1.220 <noreply@anthropic.com>
```

If `claude` is not installed and the command fails, the `fallback = ""` keeps
the trailer alive with an empty field; unknown placeholders vanish, so the name
becomes just `Claude`.

## Example: Nvim

```toml
[[co_authors]]
name = "Nvim {nvim_version}"
email = "editor@user.invalid"

[co_authors.fields]
nvim_version = { command = "nvim --version | cut -d' ' -f2 | cut -d$'\n' -f1", fallback = ""}
```

The first line of `nvim --version` is `NVIM v0.12.4`; the pipeline extracts
`v0.12.4` and renders:

```text
Co-Authored-By: Nvim v0.12.4 <editor@user.invalid>
```

Note: blamefetch already trims command output, so the trailing-newline cut is
redundant but harmless. The `$'...'` syntax requires a shell that supports
ANSI-C quoting (bash and zsh do; plain POSIX `sh` implementations such as dash
do not).

## Example: Git

```toml
[[co_authors]]
name = "Git {git_version}"
email = "git@vcs.invalid"

[co_authors.fields]
git_version = { command = "git --version | cut -d' ' -f3", fallback = ""}
```

`git --version` prints `git version 2.54.0`; the command extracts `2.54.0` and
renders:

```text
Co-Authored-By: Git 2.54.0 <git@vcs.invalid>
```

## Combining with built-in sources

You can also customize a built-in source instead of writing a config-only
entry. The source still supplies its fields; only `name` and `email` are
overridden:

```toml
[[co_authors]]
kind = "gpu"
name = "GPU: {name}"
email = "hardware@system.invalid"
```

To disable a source without removing it, set `enabled = false`:

```toml
[[co_authors]]
kind = "hostname"
enabled = false
```

Use `blank_line_before = true` to separate groups of trailers, for example when
distinguishing machine credits from tool credits:

```toml
[[co_authors]]
blank_line_before = true
name = "Claude {version}"
email = "noreply@anthropic.com"
```

## Safety

Commands configured in `fields`, `name`, or `email` run with your user's
permissions and are not sandboxed. Only configure commands you trust, and only
run config files whose contents you have reviewed. See the security section of
the [README](../README.md#security-and-liability).
