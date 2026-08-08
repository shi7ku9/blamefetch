use std::collections::HashSet;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};

/// Child processes still running, keyed by PID. Registered while a command is
/// awaited so that `kill_running_children` can reap a timed-out section's
/// command instead of leaving it as an orphan.
static RUNNING_CHILDREN: LazyLock<Mutex<HashSet<u32>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

/// Set once the run has given up on timed-out sections. A worker that starts
/// after this point must not spawn anything: its section was already reported
/// as skipped, so a late spawn would only create an orphan.
static CANCELLED: AtomicBool = AtomicBool::new(false);

/// Spawns a child and registers it under the same lock as the kill scan, so
/// no command can appear after `kill_running_children` has looked. On Unix the
/// child gets its own process group, letting the kill cover `sh -c` pipelines
/// instead of only the direct child.
fn spawn_tracked(program: &str, args: &[&str]) -> Option<Child> {
    let mut command = Command::new(program);
    command
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut guard = RUNNING_CHILDREN.lock().unwrap();
    if CANCELLED.load(Ordering::Acquire) {
        return None;
    }
    let child = command.spawn().ok()?;
    guard.insert(child.id());
    drop(guard);
    Some(child)
}

/// Waits for a spawned child, keeping it registered while it runs. Returns
/// trimmed stdout, or `None` on spawn failure / non-zero exit / non-UTF-8
/// output. Empty output is NOT filtered here — callers decide (fail-closed
/// treats empty as failure).
fn run_output(child: Child) -> Option<String> {
    let pid = child.id();
    let out = child.wait_with_output().ok();
    RUNNING_CHILDREN.lock().unwrap().remove(&pid);
    out.filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
}

pub fn cmd_output(program: &str, args: &[&str]) -> Option<String> {
    run_output(spawn_tracked(program, args)?)
}

/// Run a user-supplied command through the platform shell so pipes/globs work.
/// `sh -c` on Unix, `cmd /C` on Windows. Returns trimmed stdout, or `None`
/// on spawn failure / non-zero exit / non-UTF-8 output. Empty output is NOT
/// filtered here — callers decide (fail-closed treats empty as failure).
pub fn sh_output(command: &str) -> Option<String> {
    let (program, flag): (&str, &str) = if cfg!(windows) {
        ("cmd", "/C")
    } else {
        ("sh", "-c")
    };
    run_output(spawn_tracked(program, &[flag, command])?)
}

/// True once the run has given up on timed-out sections. Callers use it to
/// distinguish "the command was cancelled" from "the command failed", so a
/// cancelled command does not produce a second, misleading warning.
pub fn is_cancelled() -> bool {
    CANCELLED.load(Ordering::Acquire)
}

/// Kills every command still running. Call after section resolution: anything
/// still registered belongs to a section that timed out, and without this the
/// child would outlive us as an orphan. Kills the whole process group, so
/// compound `sh -c` pipelines are covered too. Signals, not privileges: this
/// is cleanup, not a security boundary.
///
/// Tradeoff: children run in their own process group, so a terminal Ctrl-C
/// (SIGINT to the foreground group) no longer reaches them — interrupting
/// blamefetch while a command runs leaves that command running. Cleaning up on
/// SIGINT would need a signal handler; accepted in exchange for pipeline
/// coverage.
#[cfg(unix)]
pub fn kill_running_children() {
    let children = RUNNING_CHILDREN.lock().unwrap();
    CANCELLED.store(true, Ordering::Release);
    for &pid in children.iter() {
        // Kill the process group first (covers `sh -c` pipelines); the direct
        // PID afterwards is redundant insurance — the leader is in its own
        // group — but harmless if the group is already gone.
        // SAFETY: kill(2) on the PID/process group of our own spawned child.
        unsafe {
            libc::kill(-(pid as libc::pid_t), libc::SIGKILL);
            libc::kill(pid as libc::pid_t, libc::SIGKILL);
        }
    }
}

#[cfg(not(unix))]
pub fn kill_running_children() {
    // No portable kill-by-PID from std on Windows; mark the run cancelled so
    // workers that start after the timeout do not spawn new children. The
    // orphan window for already-running children stays a documented residual.
    CANCELLED.store(true, Ordering::Release);
}

pub fn format_bytes(bytes: u64) -> String {
    format!("{:.2} GiB", bytes as f64 / 1024.0 / 1024.0 / 1024.0)
}

pub fn format_duration(secs: u64) -> String {
    let days = secs / 86400;
    let hours = (secs % 86400) / 3600;
    let mins = ((secs % 3600) / 60).max(1);
    match (days, hours) {
        (d, _) if d > 0 => format!("{d} days, {hours} hours, {mins} mins"),
        (0, h) if h > 0 => format!("{h} hours, {mins} mins"),
        _ => format!("{mins} mins"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_bytes_gi_b() {
        assert_eq!(format_bytes(0), "0.00 GiB");
        assert_eq!(format_bytes(1073741824), "1.00 GiB");
        assert_eq!(format_bytes(1610612736), "1.50 GiB");
    }

    #[test]
    fn format_duration_days_hours_mins() {
        assert_eq!(
            format_duration(5 * 86400 + 22 * 3600 + 34 * 60),
            "5 days, 22 hours, 34 mins"
        );
        assert_eq!(format_duration(22 * 3600 + 34 * 60), "22 hours, 34 mins");
        assert_eq!(format_duration(34 * 60), "34 mins");
    }

    #[test]
    fn cmd_output_success_and_missing() {
        assert_eq!(cmd_output("echo", &["hi"]), Some("hi".to_string()));
        assert_eq!(cmd_output("blamefetch-no-such-binary", &[]), None);
    }

    #[cfg(unix)]
    #[test]
    fn sh_output_trims_and_preserves_internal_newlines() {
        assert_eq!(sh_output("printf 'a\\nb'"), Some("a\nb".to_string()));
    }

    #[cfg(unix)]
    #[test]
    fn sh_output_supports_pipes() {
        assert_eq!(
            sh_output("printf 'hello' | tr a-z A-Z"),
            Some("HELLO".to_string())
        );
    }

    #[cfg(unix)]
    #[test]
    fn sh_output_nonzero_exit_is_none() {
        assert_eq!(sh_output("false"), None);
    }

    #[cfg(unix)]
    #[test]
    fn sh_output_missing_binary_is_none() {
        assert_eq!(sh_output("blamefetch-no-such-binary"), None);
    }

    #[cfg(unix)]
    #[test]
    fn sh_output_trims_whitespace() {
        assert_eq!(sh_output("printf '  padded  '"), Some("padded".to_string()));
    }
}
