use std::collections::HashSet;
use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{LazyLock, Mutex};

/// Cap on the bytes collected from a child's stdout before the child is
/// killed. Two tiers: internal probes (`git rev-list --all` can legitimately
/// be tens of MB on a very large repository) versus user-config commands,
/// which are untrusted and must not be able to exhaust memory — `yes` at
/// ~3 GB/s would otherwise accumulate ~16 GB during a default 5 s timeout.
const MAX_PROBE_OUTPUT_BYTES: u64 = 64 * 1024 * 1024;
const MAX_COMMAND_OUTPUT_BYTES: u64 = 1024 * 1024;

/// Child processes still running, keyed by PID. Registered while a command is
/// awaited so that `kill_running_children` can reap a timed-out section's
/// command instead of leaving it as an orphan.
static RUNNING_CHILDREN: LazyLock<Mutex<HashSet<u32>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

/// Set once the run has given up on timed-out sections. A worker that starts
/// after this point must not spawn anything: its section was already reported
/// as skipped, so a late spawn would only create an orphan.
static CANCELLED: AtomicBool = AtomicBool::new(false);

/// Signal number of the interrupt that terminated the run, if any.
static INTERRUPTED: AtomicI32 = AtomicI32::new(0);

/// Installs SIGINT/SIGTERM handlers plus a watcher that kills every tracked
/// child and exits with 128+signal. Children now run in their own process
/// group, so a terminal interrupt would otherwise leave them running.
#[cfg(unix)]
pub fn install_interrupt_cleanup() {
    extern "C" fn on_signal(sig: libc::c_int) {
        INTERRUPTED.store(sig, Ordering::Relaxed);
    }
    // SAFETY: installs a handler that only stores to an atomic (async-signal-
    // safe); SIGINT/SIGTERM are always valid signals.
    unsafe {
        libc::signal(libc::SIGINT, on_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGTERM, on_signal as *const () as libc::sighandler_t);
    }
    std::thread::spawn(|| {
        loop {
            std::thread::sleep(std::time::Duration::from_millis(50));
            let sig = INTERRUPTED.load(Ordering::Relaxed);
            if sig != 0 {
                kill_running_children();
                std::process::exit(128 + sig);
            }
        }
    });
}

#[cfg(not(unix))]
pub fn install_interrupt_cleanup() {
    // No POSIX signals to install; the platform's default Ctrl-C handling
    // stays in effect.
}

/// Spawns a child and registers it under the same lock as the kill scan, so
/// no command can appear after `kill_running_children` has looked. On Unix the
/// child gets its own process group, letting the kill cover `sh -c` pipelines
/// instead of only the direct child.
fn spawn_tracked(program: &str, args: &[&str]) -> Option<Child> {
    let mut command = Command::new(program);
    command
        .args(args)
        // Probes never read input; nulling stdin stops a config command that
        // reads it from stealing the terminal or consuming the caller's pipe.
        .stdin(Stdio::null())
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
/// output / output larger than `max_output` bytes (the child is killed in
/// that case). Empty output is NOT filtered here — callers decide (fail-
/// closed treats empty as failure).
fn run_output(mut child: Child, max_output: u64) -> Option<String> {
    let pid = child.id();

    // Drain stderr on a helper thread so a chatty child cannot deadlock on a
    // full stderr pipe while we read stdout; the bytes are discarded (never
    // surfaced). The thread unblocks as soon as the child exits or is killed.
    let stderr = child.stderr.take();
    let stderr_thread = stderr.map(|mut err| {
        std::thread::spawn(move || {
            let mut sink = std::io::sink();
            let _ = std::io::copy(&mut err, &mut sink);
        })
    });

    // Read stdout with a hard byte cap. A misbehaving command (e.g. `yes`)
    // must not be able to exhaust memory: once the cap is hit the whole
    // process group is killed and the section fails closed.
    let mut stdout = Vec::new();
    let mut failed = false;
    if let Some(mut pipe) = child.stdout.take() {
        let mut buf = [0u8; 8192];
        loop {
            match pipe.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let n = n as u64;
                    if stdout.len() as u64 + n > max_output {
                        failed = true;
                        break;
                    }
                    stdout.extend_from_slice(&buf[..n as usize]);
                }
                Err(_) => {
                    failed = true;
                    break;
                }
            }
        }
    }
    if failed {
        kill_group(pid);
    }
    let status = child.wait().ok();
    if let Some(thread) = stderr_thread {
        let _ = thread.join();
    }
    RUNNING_CHILDREN.lock().unwrap().remove(&pid);
    if failed {
        return None;
    }
    status
        .filter(|s| s.success())
        .and_then(|_| String::from_utf8(stdout).ok())
        .map(|s| s.trim().to_string())
}

pub fn cmd_output(program: &str, args: &[&str]) -> Option<String> {
    run_output(spawn_tracked(program, args)?, MAX_PROBE_OUTPUT_BYTES)
}

/// Run a user-supplied command through the platform shell so pipes/globs work.
/// `sh -c` on Unix, `cmd /C` on Windows. Returns trimmed stdout, or `None`
/// on spawn failure / non-zero exit / non-UTF-8 output / output over the
/// command cap. Empty output is NOT filtered here — callers decide (fail-
/// closed treats empty as failure).
pub fn sh_output(command: &str) -> Option<String> {
    let (program, flag): (&str, &str) = if cfg!(windows) {
        ("cmd", "/C")
    } else {
        ("sh", "-c")
    };
    run_output(
        spawn_tracked(program, &[flag, command])?,
        MAX_COMMAND_OUTPUT_BYTES,
    )
}

/// True once the run has given up on timed-out sections. Callers use it to
/// distinguish "the command was cancelled" from "the command failed", so a
/// cancelled command does not produce a second, misleading warning.
pub fn is_cancelled() -> bool {
    CANCELLED.load(Ordering::Acquire)
}

/// SIGKILLs one spawned child's process group (covers `sh -c` pipelines)
/// plus the direct PID as redundant insurance — the leader is in its own
/// group, but the direct kill is harmless if the group is already gone.
#[cfg(unix)]
fn kill_group(pid: u32) {
    // SAFETY: kill(2) on the PID/process group of our own spawned child.
    unsafe {
        libc::kill(-(pid as libc::pid_t), libc::SIGKILL);
        libc::kill(pid as libc::pid_t, libc::SIGKILL);
    }
}

#[cfg(not(unix))]
fn kill_group(_pid: u32) {
    // No portable kill-by-PID from std on Windows.
}

/// Kills every command still running. Call after section resolution: anything
/// still registered belongs to a section that timed out, and without this the
/// child would outlive us as an orphan. Kills the whole process group, so
/// compound `sh -c` pipelines are covered too. Signals, not privileges: this
/// is cleanup, not a security boundary.
///
/// Children run in their own process group, so a terminal Ctrl-C (SIGINT to
/// the foreground group) does not reach them; [`install_interrupt_cleanup`]
/// covers the interrupt path with a signal handler plus a watcher that calls
/// this before exiting with 128+signal.
#[cfg(unix)]
pub fn kill_running_children() {
    let children = RUNNING_CHILDREN.lock().unwrap();
    CANCELLED.store(true, Ordering::Release);
    for &pid in children.iter() {
        kill_group(pid);
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
    let mins = (secs % 3600) / 60;
    match (days, hours, mins) {
        (d, _, _) if d > 0 => format!("{d} days, {hours} hours, {mins} mins"),
        (0, h, _) if h > 0 => format!("{h} hours, {mins} mins"),
        (0, 0, m) if m > 0 => format!("{m} mins"),
        _ => "0 mins".to_string(),
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
    fn format_duration_boundaries() {
        // A unit that is exactly zero must not be promoted to 1 by `max(1)`.
        assert_eq!(format_duration(0), "0 mins");
        assert_eq!(format_duration(59), "0 mins");
        assert_eq!(format_duration(3600), "1 hours, 0 mins");
        assert_eq!(format_duration(3660), "1 hours, 1 mins");
        assert_eq!(format_duration(86400), "1 days, 0 hours, 0 mins");
    }

    #[cfg(unix)]
    #[test]
    fn sh_output_caps_runaway_output() {
        // An unbounded producer must be cut off and fail closed, not buffered
        // into memory. `:` and `printf` are shell builtins, so the loop needs
        // no external binary.
        assert_eq!(sh_output("while :; do printf '1234567890'; done"), None);
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
