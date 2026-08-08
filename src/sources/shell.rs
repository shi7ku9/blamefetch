use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::path::Path;

use sysinfo::{Pid, System};

use crate::util::cmd_output;

use super::{Source, SourceContext};

pub struct Shell;

impl Source for Shell {
    fn kind(&self) -> &'static str {
        "shell"
    }
    fn default_name(&self) -> &'static str {
        "{name} {version}"
    }
    fn default_email(&self) -> &'static str {
        "shell@system.invalid"
    }

    fn fields(&self, ctx: &SourceContext) -> Option<HashMap<String, String>> {
        // The shell actually running this process comes first; `$SHELL` (the
        // login shell, which may differ) is only a fallback.
        let path = detect_running_shell(&ctx.sys)
            .or_else(|| ctx.env("SHELL"))
            .filter(|p| !p.is_empty())?;
        // Normalize for display and version lookup: strip a Windows `.exe`
        // suffix and lowercase, so `bash.exe` and `bash` behave identically.
        let name = shell_stem(&Path::new(&path).file_name()?.to_string_lossy());
        if name.is_empty() {
            return None;
        }
        let mut fields = HashMap::new();
        fields.insert("name".to_string(), name.clone());
        fields.insert("version".to_string(), shell_version(&name, &path));
        Some(fields)
    }
}

/// Maximum number of ancestors to inspect before giving up. Guards against
/// malformed or cyclic parent tables; a real chain is only a few levels deep.
const MAX_ANCESTOR_DEPTH: usize = 32;

/// Executable basenames of shells recognized as "the running shell". The first
/// matching ancestor wins. Launchers/wrappers (`sudo`, `tmux`, `cargo`, …) are
/// deliberately absent so the walk passes through them to the real shell.
const KNOWN_SHELLS: &[&str] = &[
    "bash",
    "rbash",
    "sh",
    "dash",
    "ash",
    "yash",
    "zsh",
    "fish",
    "ksh",
    "mksh",
    "oksh",
    "pdksh",
    "ksh93",
    "tcsh",
    "csh",
    "nu",
    "elvish",
    "xonsh",
    "ion",
    "oil",
    "pwsh",
    "powershell",
    "cmd",
    "busybox",
];

/// Normalized executable name of a shell: ASCII-lowercased with a Windows
/// `.exe` suffix stripped (`PWSH.EXE` → `pwsh`) and a leading `-` removed so
/// login-shell argv[0] (`-zsh`) matches. Used for detection, display, and
/// version-strategy lookup so all three agree.
fn shell_stem(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    let stem = lower.strip_suffix(".exe").unwrap_or(&lower);
    stem.strip_prefix('-').unwrap_or(stem).to_string()
}

/// True when `name` is a known shell basename (normalized via [`shell_stem`]).
fn is_known_shell_name(name: &str) -> bool {
    KNOWN_SHELLS.contains(&shell_stem(name).as_str())
}

/// Walks the parent-process chain starting at `own_pid` and returns the
/// executable path of the nearest ancestor whose basename is a known shell.
/// Returns `None` when no ancestor matches, the chain ends, a cycle is
/// detected, or the depth limit is hit.
fn find_shell_in_chain(
    own_pid: u32,
    parent_of: impl Fn(u32) -> Option<u32>,
    exe_of: impl Fn(u32) -> Option<String>,
) -> Option<String> {
    let mut visited = HashSet::new();
    let mut pid = own_pid;
    for _ in 0..MAX_ANCESTOR_DEPTH {
        if pid == 0 || !visited.insert(pid) {
            return None; // end of chain or cycle
        }
        if let Some(path) = exe_of(pid) {
            let name = path.rsplit(['/', '\\']).next().unwrap_or(&path);
            if is_known_shell_name(name) {
                return Some(path); // full path, so callers can exec `--version`
            }
        }
        pid = parent_of(pid)?;
    }
    None
}

/// Best-effort executable path of a process: `exe` first, falling back to the
/// first element of `cmd` (cmdline stays readable where the exe link is not,
/// e.g. under `hidepid`). Empty values are treated as absent.
fn process_exe_path(exe: Option<&Path>, cmd: &[OsString]) -> Option<String> {
    exe.map(|p| p.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .or_else(|| cmd.first().map(|c| c.to_string_lossy().into_owned()))
        .filter(|s| !s.is_empty())
}

/// Detects the shell running this process from the process-table snapshot in
/// `sys`: walks the parent chain of our own PID and returns the nearest known
/// shell's executable path. `None` means "could not detect".
fn detect_running_shell(sys: &System) -> Option<String> {
    let processes = sys.processes();
    let parent_of = |pid: u32| {
        processes
            .get(&Pid::from_u32(pid))
            .and_then(|p| p.parent())
            .map(|p| p.as_u32())
    };
    let exe_of = |pid: u32| {
        processes
            .get(&Pid::from_u32(pid))
            .and_then(|p| process_exe_path(p.exe(), p.cmd()))
    };
    find_shell_in_chain(std::process::id(), parent_of, exe_of)
}

/// How a shell's version is read. The extension point: future shells may use
/// other strategies (e.g. a version environment variable) instead of a flag.
enum VersionStrategy {
    /// Run `<shell> <flags>` and parse the first output line.
    Flag(&'static [&'static str]),
}

/// Picks a version-reading strategy for a shell by name (normalized via
/// [`shell_stem`], so `bash.exe` is treated like `bash`). Shells outside the
/// table get no version (the name is still reported) and nothing is executed.
fn version_strategy(name: &str) -> Option<VersionStrategy> {
    match shell_stem(name).as_str() {
        "bash" | "zsh" | "fish" => Some(VersionStrategy::Flag(&["--version"])),
        _ => None,
    }
}

fn shell_version(name: &str, path: &str) -> String {
    match version_strategy(name) {
        // Only execute `--version` for a path we can run as given. A bare name
        // (e.g. from the `cmd[0]` fallback or a `$SHELL` value like `bash`)
        // would resolve through PATH, which may point at a different binary
        // than the detected ancestor.
        Some(VersionStrategy::Flag(args)) if path.contains(['/', '\\']) => {
            flag_version(path, args).unwrap_or_default()
        }
        _ => String::new(),
    }
}

/// Runs `<path> <flags>` and parses the version from the first output line.
fn flag_version(path: &str, flags: &[&str]) -> Option<String> {
    let out = cmd_output(path, flags)?;
    let line = out.lines().next()?;
    parse_shell_version(line)
}

/// Extracts a version from a shell's `--version` first line without assuming a
/// particular shell's output shape. Returns the first whitespace-separated
/// token that starts with a digit, truncated at an opening parenthesis (so
/// bash's `5.3.9(1)-release` becomes `5.3.9`).
fn parse_shell_version(line: &str) -> Option<String> {
    line.split_whitespace()
        .find(|t| t.chars().next().is_some_and(|c| c.is_ascii_digit()))
        .map(|t| t.split('(').next().unwrap().to_string())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::ffi::OsString;
    use std::path::Path;

    use sysinfo::System;

    use super::{
        detect_running_shell, find_shell_in_chain, is_known_shell_name, parse_shell_version,
        process_exe_path, shell_version, version_strategy,
    };

    /// Walks a fabricated chain of `(pid, parent, exe)` rows with the pure
    /// `find_shell_in_chain` function.
    fn walk(own_pid: u32, procs: &[(u32, Option<u32>, Option<&str>)]) -> Option<String> {
        let by_pid: HashMap<u32, (Option<u32>, Option<&str>)> = procs
            .iter()
            .map(|(p, par, exe)| (*p, (*par, *exe)))
            .collect();
        find_shell_in_chain(
            own_pid,
            |pid| by_pid.get(&pid).and_then(|(par, _)| *par),
            |pid| {
                by_pid
                    .get(&pid)
                    .and_then(|(_, exe)| exe.map(str::to_string))
            },
        )
    }

    #[test]
    fn first_shell_ancestor_wins() {
        let procs = [
            (100, Some(99), Some("/usr/bin/blamefetch")),
            (99, Some(98), Some("/usr/bin/cargo")),
            (98, Some(97), Some("/usr/bin/bash")),
            (97, None, Some("/usr/bin/zsh")),
        ];
        assert_eq!(walk(100, &procs), Some("/usr/bin/bash".to_string()));
    }

    #[test]
    fn skips_non_shell_ancestors() {
        let procs = [
            (100, Some(99), Some("/usr/bin/blamefetch")),
            (99, Some(98), Some("/usr/bin/cargo")),
            (98, None, Some("/usr/bin/zsh")),
        ];
        assert_eq!(walk(100, &procs), Some("/usr/bin/zsh".to_string()));
    }

    #[test]
    fn no_shell_in_chain_is_none() {
        let procs = [
            (100, Some(99), Some("/usr/bin/blamefetch")),
            (99, Some(98), Some("/usr/bin/cargo")),
            (98, Some(97), Some("/usr/bin/init")),
            (97, None, Some("/usr/bin/init")),
        ];
        assert_eq!(walk(100, &procs), None);
    }

    #[test]
    fn stops_at_missing_parent() {
        assert_eq!(walk(100, &[]), None);
    }

    #[test]
    fn cycle_in_parent_chain_is_none() {
        // The cycle contains no shell, so it must be detected before a match.
        let procs = [
            (100, Some(101), Some("/usr/bin/blamefetch")),
            (101, Some(100), Some("/usr/bin/cargo")),
        ];
        assert_eq!(walk(100, &procs), None);
    }

    #[test]
    fn self_parent_is_none() {
        let procs = [(100, Some(100), Some("/usr/bin/blamefetch"))];
        assert_eq!(walk(100, &procs), None);
    }

    #[test]
    fn depth_limit_guard() {
        // A known shell lurks at depth 40: without the depth cap the walk
        // would return it; with the cap (32) the loop bound fires first.
        let mut procs: Vec<(u32, Option<u32>, Option<&str>)> = (0..41)
            .map(|i| (100 - i, Some(99 - i), Some("/usr/bin/not-a-shell")))
            .collect();
        procs[40] = (60, None, Some("/usr/bin/zsh"));
        assert_eq!(walk(100, &procs), None);
    }

    #[test]
    fn own_pid_is_a_shell_is_found_first() {
        // A wrapper that `exec`'d into a shell: the process's own exe is the
        // shell, so the walk returns it immediately.
        let procs = [(100, None, Some("/usr/bin/zsh"))];
        assert_eq!(walk(100, &procs), Some("/usr/bin/zsh".to_string()));
    }

    #[test]
    fn windows_paths_are_matched() {
        let procs = [
            (100, Some(99), Some("C:\\tools\\blamefetch.exe")),
            (99, None, Some("C:\\Program Files\\PowerShell\\7\\pwsh.exe")),
        ];
        assert_eq!(
            walk(100, &procs),
            Some("C:\\Program Files\\PowerShell\\7\\pwsh.exe".to_string())
        );
    }

    #[test]
    fn known_shell_name_matching() {
        for name in [
            "zsh", "bash", "rbash", "yash", "dash", "-zsh", "-bash", "cmd.exe", "pwsh.exe",
            "PWSH.EXE",
        ] {
            assert!(is_known_shell_name(name), "{name} should be a shell");
        }
        for name in ["cargo", "blamefetch", "tmux"] {
            assert!(!is_known_shell_name(name), "{name} should not be a shell");
        }
    }

    #[test]
    fn process_exe_path_fallback() {
        assert_eq!(
            process_exe_path(None, &[OsString::from("zsh"), OsString::from("-c")]),
            Some("zsh".to_string())
        );
        assert_eq!(
            process_exe_path(Some(Path::new("/usr/bin/zsh")), &[OsString::from("zsh")]),
            Some("/usr/bin/zsh".to_string())
        );
        assert_eq!(process_exe_path(None, &[]), None);
        // Empty exe falls through to cmd[0].
        assert_eq!(
            process_exe_path(Some(Path::new("")), &[OsString::from("zsh")]),
            Some("zsh".to_string())
        );
    }

    #[test]
    fn detect_running_shell_on_empty_system_is_none() {
        // sysinfo cannot be populated with fake processes, so the adapter's
        // `Some` path (Pid wiring, exe→cmd[0] fallback) is only covered by
        // manual end-to-end runs.
        assert!(detect_running_shell(&System::new()).is_none());
    }

    #[test]
    fn version_strategy_known_shells() {
        for name in ["bash", "zsh", "fish", "bash.exe", "BASH.EXE"] {
            assert!(
                version_strategy(name).is_some(),
                "{name} should have a strategy"
            );
        }
    }

    #[test]
    fn shell_version_bare_name_is_skipped() {
        // A bare name from the `cmd[0]` fallback must not resolve through PATH.
        assert_eq!(shell_version("zsh", "zsh"), "");
    }

    #[test]
    fn shell_version_unexecutable_path_is_empty() {
        // A real path runs, but a missing binary fails fail-closed.
        assert_eq!(shell_version("zsh", "/nonexistent/blamefetch-zsh"), "");
    }

    #[test]
    fn version_strategy_unknown_is_none() {
        for name in ["sh", "dash", "nu", "cargo"] {
            assert!(
                version_strategy(name).is_none(),
                "{name} should have no strategy"
            );
        }
    }

    #[test]
    fn parses_zsh_style_version() {
        assert_eq!(
            parse_shell_version("zsh 5.9.1 (x86_64-pc-linux-gnu)"),
            Some("5.9.1".to_string())
        );
    }

    #[test]
    fn parses_bash_style_version() {
        assert_eq!(
            parse_shell_version("GNU bash, version 5.3.9(1)-release (x86_64-pc-linux-gnu)"),
            Some("5.3.9".to_string())
        );
    }

    #[test]
    fn parses_fish_style_version() {
        assert_eq!(
            parse_shell_version("fish, version 3.7.1"),
            Some("3.7.1".to_string())
        );
    }

    #[test]
    fn no_version_token_is_none() {
        assert_eq!(parse_shell_version("Some Shell"), None);
        assert_eq!(parse_shell_version(""), None);
    }
}
