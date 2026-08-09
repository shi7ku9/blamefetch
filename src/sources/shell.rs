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
        // suffix, lowercase, and drop a digit-led version suffix, so
        // `bash.exe`, `zsh5` and `zsh` all behave as `bash`/`zsh`.
        let name = strip_version_suffix(&path_stem(&path)?).to_string();
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

/// Strips a trailing digit-led version suffix from a normalized shell name:
/// `zsh5` → `zsh`, `zsh-5.9` → `zsh`, `zsh-5.9-devel` → `zsh`. Names without
/// a digit-led tail (`zshrc`, `bash-static`, `busybox`) are returned
/// unchanged. This is what makes versioned distro binaries (Debian's
/// `/usr/bin/zsh` → `zsh5`) match the same detection/strategy entries as
/// their unversioned name.
fn strip_version_suffix(name: &str) -> &str {
    for (i, c) in name.char_indices() {
        if c.is_ascii_digit() && i > 0 {
            // Cut at the preceding separator when there is one
            // (`zsh-5.9` → `zsh`), else at the digit itself (`zsh5` → `zsh`).
            let cut = if matches!(name.as_bytes()[i - 1], b'-' | b'.') {
                i - 1
            } else {
                i
            };
            return &name[..cut];
        }
    }
    name
}

/// Basename of a path normalized via [`shell_stem`], or `None` when the path
/// has no usable basename. Splits on both separators (not `Path::file_name`,
/// which on Unix treats `\` as an ordinary character and would return the
/// whole of a Windows-style path as the "file name").
fn path_stem(path: &str) -> Option<String> {
    let base = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let stem = shell_stem(base);
    if stem.is_empty() { None } else { Some(stem) }
}

/// True when `name` is a known shell basename (normalized via [`shell_stem`]
/// with any version suffix stripped, so `zsh5` matches like `zsh`).
fn is_known_shell_name(name: &str) -> bool {
    let stem = shell_stem(name);
    KNOWN_SHELLS.contains(&strip_version_suffix(&stem))
}

/// Walks the parent-process chain starting at `own_pid` and returns the
/// program path of the nearest ancestor whose basename is a known shell.
/// Returns `None` when no ancestor matches, the chain ends, a cycle is
/// detected, or the depth limit is hit.
fn find_shell_in_chain(
    own_pid: u32,
    parent_of: impl Fn(u32) -> Option<u32>,
    program_paths_of: impl Fn(u32) -> Vec<String>,
) -> Option<String> {
    let mut visited = HashSet::new();
    let mut pid = own_pid;
    for _ in 0..MAX_ANCESTOR_DEPTH {
        if pid == 0 || !visited.insert(pid) {
            return None; // end of chain or cycle
        }
        for path in program_paths_of(pid) {
            let name = path.rsplit(['/', '\\']).next().unwrap_or(&path);
            if is_known_shell_name(name) {
                return Some(path); // full path, so callers can exec `--version`
            }
        }
        pid = parent_of(pid)?;
    }
    None
}

/// Program-path candidates of a process, most trustworthy first: the real
/// executable, then the first two command-line words. The shell name is not
/// always visible in `exe` — nixpkgs wraps pwsh as `.pwsh-wrapped`, and
/// shebang shells like xonsh run as `python3 /usr/bin/xonsh` — but the
/// command line still carries it. `cmd[0]` alone also covers `hidepid`, where
/// the exe link is unreadable but the command line is not. Empty values are
/// dropped.
///
/// What the version probe guarantees: it only runs a returned path that
/// contains a path separator AND whose basename is a known shell name, so a
/// bare name is never resolved through PATH. That is not a security boundary
/// though: a parent process controls its own argv, so it can name any file
/// like a shell (e.g. `python3 /tmp/evil/bash`) and have the `--version`
/// probe run it. Normally that parent and this process share the same
/// privileges, so nothing is gained; if blamefetch runs elevated (via `sudo`,
/// a service manager, or setuid), the probe executes input from a
/// lower-privileged parent chain and must be treated as untrusted.
fn process_shell_candidates(exe: Option<&Path>, cmd: &[OsString]) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(exe) = exe.filter(|p| !p.as_os_str().is_empty()) {
        out.push(exe.to_string_lossy().into_owned());
    }
    for arg in cmd.iter().take(2) {
        let s = arg.to_string_lossy().into_owned();
        if !s.is_empty() && !out.contains(&s) {
            out.push(s);
        }
    }
    out
}

/// Detects the shell running this process from the process-table snapshot in
/// `sys`: walks the parent chain of our own PID and returns the nearest known
/// shell's program path. `None` means "could not detect".
fn detect_running_shell(sys: &System) -> Option<String> {
    let processes = sys.processes();
    let parent_of = |pid: u32| {
        processes
            .get(&Pid::from_u32(pid))
            .and_then(|p| p.parent())
            .map(|p| p.as_u32())
    };
    let program_paths_of = |pid: u32| {
        processes
            .get(&Pid::from_u32(pid))
            .map(|p| process_shell_candidates(p.exe(), p.cmd()))
            .unwrap_or_default()
    };
    find_shell_in_chain(std::process::id(), parent_of, program_paths_of)
}

/// How a shell's version is read. The extension point: future shells may use
/// other strategies (e.g. a version environment variable) instead of a flag.
enum VersionStrategy {
    /// Run `<shell> <flags>` and parse the first output line.
    Flag(&'static [&'static str]),
}

/// Picks a version-reading strategy for a shell by name (normalized via
/// [`shell_stem`] with the version suffix stripped, so `bash.exe` and
/// `zsh5` are treated like `bash` and `zsh`). Shells outside the table get
/// no version (the name is still reported) and nothing is executed.
fn version_strategy(name: &str) -> Option<VersionStrategy> {
    let stem = shell_stem(name);
    match strip_version_suffix(&stem) {
        "bash" | "zsh" | "fish" | "nu" | "elvish" | "xonsh" | "pwsh" => {
            Some(VersionStrategy::Flag(&["--version"]))
        }
        _ => None,
    }
}

fn shell_version(name: &str, path: &str) -> String {
    match version_strategy(name) {
        // Only execute `--version` for a path we can run as given AND that
        // still looks like a shell after symlink resolution AND when probing
        // was not explicitly disabled. A bare name (e.g. from the `cmd[0]`
        // fallback or a `$SHELL` value like `bash`) would resolve through
        // PATH, which may point at a different binary than the detected
        // ancestor.
        Some(VersionStrategy::Flag(args))
            if path.contains(['/', '\\'])
                && !version_probing_disabled(
                    std::env::var_os("BLAMEFETCH_NO_SHELL_VERSION").as_deref(),
                )
                && is_verified_shell_path(path, name) =>
        {
            flag_version(path, args).unwrap_or_default()
        }
        _ => String::new(),
    }
}

/// True when `BLAMEFETCH_NO_SHELL_VERSION` is set to a non-empty value other
/// than "0": version probing is skipped entirely, so a detected shell path is
/// never executed. For setups where blamefetch runs elevated (via `sudo`, a
/// service manager, or setuid), this closes the "parent-provided path is
/// executed" surface completely.
fn version_probing_disabled(value: Option<&std::ffi::OsStr>) -> bool {
    value.is_some_and(|v| !v.is_empty() && v != "0")
}

/// Extra gate before executing a detected shell path: resolve symlinks and
/// re-check that the target's basename still names the same shell — exactly
/// (`bash`), or as a version-suffixed variant (`zsh-5.9`, or Debian's
/// `zsh` -> `zsh5`). This catches a symlink *named* like a shell but pointing
/// at an arbitrary file (e.g. `/usr/bin/bash` -> `/tmp/evil/bash-backdoor`).
/// It is not a full trust boundary — a parent can still name a real file
/// `bash` — so the definitive control for elevated setups is
/// `BLAMEFETCH_NO_SHELL_VERSION`.
fn is_verified_shell_path(path: &str, name: &str) -> bool {
    let Ok(target) = std::fs::canonicalize(path) else {
        return false;
    };
    let Some(stem) = path_stem(&target.to_string_lossy()) else {
        return false;
    };
    // A version suffix is a digit-led run after the name, with or without a
    // dash: `zsh5` and `zsh-5.9` are legitimate versioned binaries,
    // `bash-backdoor` and `zshrc` are not.
    stem == name
        || stem.strip_prefix(name).is_some_and(|suffix| {
            !suffix.is_empty()
                && suffix
                    .strip_prefix('-')
                    .unwrap_or(suffix)
                    .starts_with(|c: char| c.is_ascii_digit())
        })
}

/// Runs `<path> <flags>` and parses the version from the first output line.
fn flag_version(path: &str, flags: &[&str]) -> Option<String> {
    let out = cmd_output(path, flags)?;
    let line = out.lines().next()?;
    parse_shell_version(line)
}

/// Extracts a version from a shell's `--version` first line without assuming a
/// particular shell's output shape. Picks the first whitespace-separated token
/// whose version starts at its first digit — at the token start (zsh's
/// `5.9.1`), after a leading `v` (`v0.21.0`), or after a slash prefix
/// (xonsh's `xonsh/0.19.1`) — then cuts from that digit and truncates at an
/// opening parenthesis (so bash's `5.3.9(1)-release` becomes `5.3.9`).
fn parse_shell_version(line: &str) -> Option<String> {
    line.split_whitespace()
        .find(|t| {
            t.find(|c: char| c.is_ascii_digit()).is_some_and(|pos| {
                pos == 0 || (t.starts_with('v') && pos == 1) || t[..pos].contains('/')
            })
        })
        .map(|t| {
            let start = t.find(|c: char| c.is_ascii_digit()).unwrap();
            t[start..].split('(').next().unwrap().to_string()
        })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::ffi::OsString;
    use std::path::Path;

    use sysinfo::System;

    use super::{
        detect_running_shell, find_shell_in_chain, is_known_shell_name, is_verified_shell_path,
        parse_shell_version, path_stem, process_shell_candidates, shell_version,
        strip_version_suffix, version_probing_disabled, version_strategy,
    };

    /// Walks a fabricated chain of `(pid, parent, exe)` rows with the pure
    /// `find_shell_in_chain` function.
    /// One fabricated process row: `(pid, parent, exe, cmd)`.
    type ProcRow = (
        u32,
        Option<u32>,
        Option<&'static str>,
        &'static [&'static str],
    );

    fn walk(own_pid: u32, procs: &[(u32, Option<u32>, Option<&'static str>)]) -> Option<String> {
        walk_cmd(
            own_pid,
            &procs
                .iter()
                .map(|(p, par, exe)| (*p, *par, *exe, &[][..]))
                .collect::<Vec<_>>(),
        )
    }

    /// Walks a fabricated chain of [`ProcRow`]s.
    fn walk_cmd(own_pid: u32, procs: &[ProcRow]) -> Option<String> {
        /// `(parent, exe, cmd)` lookup row.
        type LookupRow = (Option<u32>, Option<&'static str>, Vec<String>);
        let by_pid: HashMap<u32, LookupRow> = procs
            .iter()
            .map(|(p, par, exe, cmd)| {
                (
                    *p,
                    (*par, *exe, cmd.iter().map(|s| s.to_string()).collect()),
                )
            })
            .collect();
        find_shell_in_chain(
            own_pid,
            |pid| by_pid.get(&pid).and_then(|(par, _, _)| *par),
            |pid| {
                by_pid
                    .get(&pid)
                    .map(|(_, exe, cmd)| {
                        let mut candidates = Vec::new();
                        if let Some(exe) = exe {
                            candidates.push(exe.to_string());
                        }
                        candidates.extend(cmd.iter().cloned());
                        candidates
                    })
                    .unwrap_or_default()
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
    fn wrapped_shell_recognized_via_cmd0() {
        // nixpkgs wraps pwsh as `.pwsh-wrapped`; the shell name only survives
        // in the command line (cmd[0]).
        let procs = [
            (
                100,
                Some(99),
                Some("/nix/store/pwsh/share/powershell/.pwsh-wrapped"),
                &["/nix/store/pwsh/bin/pwsh"][..],
            ),
            (99, None, Some("/usr/bin/zsh"), &["zsh"][..]),
        ];
        assert_eq!(
            walk_cmd(100, &procs),
            Some("/nix/store/pwsh/bin/pwsh".to_string())
        );
    }

    #[test]
    fn shebang_shell_recognized_via_cmd1() {
        // xonsh runs as `python3 /usr/bin/xonsh`: exe is the interpreter, the
        // shell script path sits at cmd[1].
        let procs = [
            (
                100,
                Some(99),
                Some("/usr/bin/python3"),
                &["python3", "/usr/bin/xonsh"][..],
            ),
            (99, None, Some("/usr/bin/zsh"), &["zsh"][..]),
        ];
        assert_eq!(walk_cmd(100, &procs), Some("/usr/bin/xonsh".to_string()));
    }

    #[test]
    fn exe_wins_over_cmd_candidates() {
        // When exe itself is a known shell, the real path wins over a crafted
        // argv[0].
        let procs = [
            (100, Some(99), Some("/usr/bin/zsh"), &["/weird/argv0"][..]),
            (99, None, Some("/usr/bin/bash"), &["bash"][..]),
        ];
        assert_eq!(walk_cmd(100, &procs), Some("/usr/bin/zsh".to_string()));
    }

    #[test]
    fn known_shell_name_matching() {
        for name in [
            "zsh", "bash", "rbash", "yash", "dash", "-zsh", "-bash", "cmd.exe", "pwsh.exe",
            "PWSH.EXE",
            // Version-suffixed distro binaries (Debian's zsh → zsh5) match
            // their unversioned name.
            "zsh5", "zsh-5.9", "bash5",
        ] {
            assert!(is_known_shell_name(name), "{name} should be a shell");
        }
        for name in ["cargo", "blamefetch", "tmux", "zshrc", "bash-static"] {
            assert!(!is_known_shell_name(name), "{name} should not be a shell");
        }
    }

    #[test]
    fn strip_version_suffix_cases() {
        assert_eq!(strip_version_suffix("zsh"), "zsh");
        assert_eq!(strip_version_suffix("zsh5"), "zsh");
        assert_eq!(strip_version_suffix("zsh-5.9"), "zsh");
        assert_eq!(strip_version_suffix("zsh-5.9-devel"), "zsh");
        assert_eq!(strip_version_suffix("bash5-evil"), "bash");
        assert_eq!(strip_version_suffix("zsh.5.9"), "zsh");
        assert_eq!(strip_version_suffix("zshrc"), "zshrc");
        assert_eq!(strip_version_suffix("bash-static"), "bash-static");
        assert_eq!(strip_version_suffix("busybox"), "busybox");
        assert_eq!(strip_version_suffix("ksh93"), "ksh");
        assert_eq!(strip_version_suffix("sh"), "sh");
        assert_eq!(strip_version_suffix(""), "");
    }

    #[test]
    fn process_shell_candidates_prefers_exe_then_cmd() {
        // The bare cmd name is kept as a second candidate (not a duplicate of
        // the full path); it only matters when exe is not a known shell.
        assert_eq!(
            process_shell_candidates(Some(Path::new("/usr/bin/zsh")), &[OsString::from("zsh")]),
            vec!["/usr/bin/zsh".to_string(), "zsh".to_string()]
        );
        // Empty exe falls through to the command line.
        assert_eq!(
            process_shell_candidates(Some(Path::new("")), &[OsString::from("zsh")]),
            vec!["zsh".to_string()]
        );
        assert_eq!(process_shell_candidates(None, &[]), Vec::<String>::new());
        // Wrapped binaries keep the shell name in cmd[0].
        assert_eq!(
            process_shell_candidates(
                Some(Path::new("/nix/store/x/share/powershell/.pwsh-wrapped")),
                &[OsString::from("/nix/store/x/bin/pwsh")]
            ),
            vec![
                "/nix/store/x/share/powershell/.pwsh-wrapped".to_string(),
                "/nix/store/x/bin/pwsh".to_string()
            ]
        );
        // Exact duplicates between exe and cmd are dropped.
        assert_eq!(
            process_shell_candidates(
                Some(Path::new("/usr/bin/zsh")),
                &[OsString::from("/usr/bin/zsh"), OsString::from("-l")]
            ),
            vec!["/usr/bin/zsh".to_string(), "-l".to_string()]
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
        for name in [
            "bash", "zsh", "fish", "nu", "elvish", "xonsh", "pwsh", "bash.exe", "BASH.EXE",
            "PWSH.EXE", // Version-suffixed names resolve to the same strategy.
            "zsh5", "zsh-5.9", "bash5",
        ] {
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

    #[cfg(unix)]
    #[test]
    fn shell_version_symlink_to_non_shell_is_skipped() {
        use std::os::unix::fs::PermissionsExt;
        // A symlink *named* like a shell but pointing at an arbitrary file
        // must not be executed for a version probe. The payload is
        // executable and prints a parseable version, so only the symlink
        // gate — not a permission error — can produce the empty version.
        let dir = tempfile::tempdir().unwrap();
        let evil = dir.path().join("evil");
        std::fs::write(&evil, "#!/bin/sh\necho zsh 5.9.1\n").unwrap();
        std::fs::set_permissions(&evil, std::fs::Permissions::from_mode(0o755)).unwrap();
        let fake = dir.path().join("zsh");
        std::os::unix::fs::symlink(&evil, &fake).unwrap();
        assert_eq!(shell_version("zsh", fake.to_str().unwrap()), "");
        assert!(!is_verified_shell_path(fake.to_str().unwrap(), "zsh"));
    }

    #[cfg(unix)]
    #[test]
    fn shell_version_prefix_named_target_is_skipped() {
        use std::os::unix::fs::PermissionsExt;
        // A payload merely *prefixed* with the shell name (`bash-backdoor`)
        // must not pass the gate either: only an exact name or a digit-led
        // version suffix is accepted.
        let dir = tempfile::tempdir().unwrap();
        let payload = dir.path().join("bash-backdoor");
        std::fs::write(&payload, "#!/bin/sh\necho bash 5.3.9\n").unwrap();
        std::fs::set_permissions(&payload, std::fs::Permissions::from_mode(0o755)).unwrap();
        let fake = dir.path().join("bash");
        std::os::unix::fs::symlink(&payload, &fake).unwrap();
        assert_eq!(shell_version("bash", fake.to_str().unwrap()), "");
        assert!(!is_verified_shell_path(fake.to_str().unwrap(), "bash"));
    }

    #[test]
    fn verified_shell_path_accepts_regular_file_named_shell() {
        let dir = tempfile::tempdir().unwrap();
        let sh = dir.path().join("sh");
        std::fs::write(&sh, "#!/bin/sh\n").unwrap();
        assert!(is_verified_shell_path(sh.to_str().unwrap(), "sh"));
    }

    #[test]
    fn verified_shell_path_accepts_version_suffixed_target() {
        let dir = tempfile::tempdir().unwrap();
        let versioned = dir.path().join("zsh-5.9");
        std::fs::write(&versioned, "#!/bin/sh\n").unwrap();
        assert!(is_verified_shell_path(versioned.to_str().unwrap(), "zsh"));
    }

    #[test]
    fn verified_shell_path_accepts_digit_suffixed_target() {
        // Debian ships zsh as /usr/bin/zsh -> zsh5 (no dash).
        let dir = tempfile::tempdir().unwrap();
        let versioned = dir.path().join("zsh5");
        std::fs::write(&versioned, "#!/bin/sh\n").unwrap();
        assert!(is_verified_shell_path(versioned.to_str().unwrap(), "zsh"));
    }

    #[test]
    fn verified_shell_path_rejects_rc_suffix() {
        // `zshrc` is not a versioned binary: a suffix that is not digit-led
        // must fail the gate even though it starts with the shell name.
        let dir = tempfile::tempdir().unwrap();
        let zshrc = dir.path().join("zshrc");
        std::fs::write(&zshrc, "#!/bin/sh\n").unwrap();
        assert!(!is_verified_shell_path(zshrc.to_str().unwrap(), "zsh"));
    }

    #[test]
    fn path_stem_normalizes_both_separators() {
        assert_eq!(path_stem("/usr/bin/zsh").as_deref(), Some("zsh"));
        assert_eq!(
            path_stem("C:\\Program Files\\PowerShell\\7\\pwsh.exe").as_deref(),
            Some("pwsh"),
            "Windows-style paths must split on backslash even on Unix"
        );
        assert_eq!(path_stem("bash").as_deref(), Some("bash"));
        assert_eq!(path_stem(""), None);
        assert_eq!(path_stem("/"), None);
        assert_eq!(path_stem("-"), None);
    }

    #[test]
    fn version_probing_disabled_semantics() {
        use std::ffi::OsStr;
        // Unset, empty, and "0" keep probing enabled; any other value disables.
        assert!(!version_probing_disabled(None));
        assert!(!version_probing_disabled(Some(OsStr::new(""))));
        assert!(!version_probing_disabled(Some(OsStr::new("0"))));
        assert!(version_probing_disabled(Some(OsStr::new("1"))));
        assert!(version_probing_disabled(Some(OsStr::new("yes"))));
    }

    #[test]
    fn version_strategy_unknown_is_none() {
        // ksh etc. have no verified `--version` shape yet — one-line follow-ups.
        for name in ["sh", "dash", "ksh", "mksh", "tcsh", "cargo"] {
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
    fn parses_nu_style_version() {
        assert_eq!(parse_shell_version("1.128.0"), Some("1.128.0".to_string()));
    }

    #[test]
    fn parses_elvish_style_version() {
        assert_eq!(parse_shell_version("0.21.0"), Some("0.21.0".to_string()));
    }

    #[test]
    fn parses_v_prefixed_version() {
        // Defensive: some distro-patched shells print `Elvish v0.21.0`.
        assert_eq!(
            parse_shell_version("Elvish v0.21.0"),
            Some("0.21.0".to_string())
        );
    }

    #[test]
    fn parses_xonsh_style_version() {
        // xonsh prints its version as one token with a slash prefix.
        assert_eq!(
            parse_shell_version("xonsh/0.19.1"),
            Some("0.19.1".to_string())
        );
    }

    #[test]
    fn parses_pwsh_style_version() {
        assert_eq!(
            parse_shell_version("PowerShell 7.5.2"),
            Some("7.5.2".to_string())
        );
    }

    #[test]
    fn parses_ksh_style_version() {
        assert_eq!(
            parse_shell_version("version sh (AT&T Research) 93u+m/1.0.9"),
            Some("93u+m/1.0.9".to_string())
        );
    }

    #[test]
    fn no_version_token_is_none() {
        assert_eq!(parse_shell_version("Some Shell"), None);
        assert_eq!(parse_shell_version(""), None);
        // A parenthesized platform tag is not a version token.
        assert_eq!(parse_shell_version("(x86_64-pc-linux-gnu)"), None);
    }
}
