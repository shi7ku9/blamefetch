use std::collections::HashMap;

use crate::util::cmd_output;

use super::{Source, SourceContext};

pub struct Kernel;

impl Source for Kernel {
    fn kind(&self) -> &'static str {
        "kernel"
    }
    fn default_name(&self) -> &'static str {
        "{name} {release}"
    }
    fn default_email(&self) -> &'static str {
        "kernel@system.invalid"
    }

    fn fields(&self, _ctx: &SourceContext) -> Option<HashMap<String, String>> {
        let release = cmd_output("uname", &["-r"])?;
        let mut fields = HashMap::new();
        fields.insert("name".to_string(), kernel_name());
        fields.insert("release".to_string(), release);
        Some(fields)
    }
}

fn kernel_name() -> String {
    match std::env::consts::OS {
        "linux" => "Linux",
        "macos" => "Darwin",
        other => other,
    }
    .to_string()
}
