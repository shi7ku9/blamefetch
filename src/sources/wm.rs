use std::collections::HashMap;

use super::{Source, SourceContext};

pub struct Wm;

impl Source for Wm {
    fn kind(&self) -> &'static str {
        "wm"
    }
    fn default_name(&self) -> &'static str {
        "{name}"
    }
    fn default_email(&self) -> &'static str {
        "wm@system.invalid"
    }

    fn fields(&self, ctx: &SourceContext) -> Option<HashMap<String, String>> {
        let name = ctx
            .env("XDG_CURRENT_DESKTOP")
            .or_else(|| ctx.env("DESKTOP_SESSION"))?;
        if name.is_empty() {
            return None;
        }
        let mut fields = HashMap::new();
        fields.insert("name".to_string(), name);
        Some(fields)
    }
}
