use std::collections::HashMap;
use std::path::Path;

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
        let path = ctx.env("SHELL")?;
        if path.is_empty() {
            return None;
        }
        let name = Path::new(&path).file_name()?.to_string_lossy().to_string();
        if name.is_empty() {
            return None;
        }
        let mut fields = HashMap::new();
        fields.insert("name".to_string(), name.clone());
        fields.insert(
            "version".to_string(),
            shell_version(&path).unwrap_or_default(),
        );
        Some(fields)
    }
}

fn shell_version(path: &str) -> Option<String> {
    let out = cmd_output(path, &["--version"])?;
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
    use super::parse_shell_version;

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
