use std::collections::HashMap;

use super::{Source, SourceContext};

pub struct Terminal;

impl Source for Terminal {
    fn kind(&self) -> &'static str {
        "terminal"
    }
    fn default_name(&self) -> &'static str {
        "{name}"
    }
    fn default_email(&self) -> &'static str {
        "terminal@system.invalid"
    }

    fn fields(&self, ctx: &SourceContext) -> Option<HashMap<String, String>> {
        let name = ctx.env("TERM")?;
        if name.is_empty() {
            return None;
        }
        let mut fields = HashMap::new();
        fields.insert("name".to_string(), name);
        Some(fields)
    }
}
