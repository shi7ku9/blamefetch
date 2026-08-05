use std::collections::HashMap;

use crate::util::cmd_output;

use super::{Source, SourceContext};

pub struct Gpu;

impl Source for Gpu {
    fn kind(&self) -> &'static str {
        "gpu"
    }
    fn default_name(&self) -> &'static str {
        "{name}"
    }
    fn default_email(&self) -> &'static str {
        "gpu@system.invalid"
    }

    fn fields(&self, _ctx: &SourceContext) -> Option<HashMap<String, String>> {
        let name = gpu_name()?;
        let mut fields = HashMap::new();
        fields.insert("name".to_string(), name);
        Some(fields)
    }
}

fn gpu_name() -> Option<String> {
    let out = cmd_output("lspci", &["-nn"])?;
    for line in out.lines() {
        let lower = line.to_lowercase();
        if lower.contains("vga compatible")
            || lower.contains("3d controller")
            || lower.contains("display controller")
        {
            let name = line.split(": ").nth(1).unwrap_or(line).trim();
            if !name.is_empty() {
                return Some(name.to_string());
            }
        }
    }
    None
}
