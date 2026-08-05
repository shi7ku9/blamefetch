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
        let name = Path::new(&path).file_name()?.to_string_lossy().to_string();
        if name.is_empty() {
            return None;
        }
        let mut fields = HashMap::new();
        fields.insert("name".to_string(), name.clone());
        fields.insert(
            "version".to_string(),
            shell_version(&name).unwrap_or_default(),
        );
        Some(fields)
    }
}

fn shell_version(name: &str) -> Option<String> {
    let out = cmd_output(name, &["--version"])?;
    let line = out.lines().next()?;
    let tokens: Vec<&str> = line.split_whitespace().collect();
    if tokens.len() >= 2 && tokens[1].chars().next().is_some_and(|c| c.is_ascii_digit()) {
        Some(tokens[1].to_string())
    } else {
        None
    }
}
