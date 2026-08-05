pub fn cmd_output(program: &str, args: &[&str]) -> Option<String> {
    std::process::Command::new(program)
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
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
    std::process::Command::new(program)
        .arg(flag)
        .arg(command)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
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
