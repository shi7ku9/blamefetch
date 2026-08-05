use std::collections::HashMap;

use super::{Source, SourceContext};

pub struct Locale;

impl Source for Locale {
    fn kind(&self) -> &'static str {
        "locale"
    }
    fn default_name(&self) -> &'static str {
        "{locale}"
    }
    fn default_email(&self) -> &'static str {
        "locale@system.invalid"
    }

    fn fields(&self, ctx: &SourceContext) -> Option<HashMap<String, String>> {
        let locale = ctx.env("LANG").or_else(|| ctx.env("LC_ALL"))?;
        if locale.is_empty() {
            return None;
        }
        let mut fields = HashMap::new();
        fields.insert("locale".to_string(), locale);
        Some(fields)
    }
}
