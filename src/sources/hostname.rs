use std::collections::HashMap;

use super::{Source, SourceContext};

pub struct Hostname;

impl Source for Hostname {
    fn kind(&self) -> &'static str {
        "hostname"
    }
    fn default_name(&self) -> &'static str {
        "{hostname}"
    }
    fn default_email(&self) -> &'static str {
        "{hostname}@host.local"
    }

    fn fields(&self, ctx: &SourceContext) -> Option<HashMap<String, String>> {
        if ctx.hostname.is_empty() {
            return None;
        }
        let mut fields = HashMap::new();
        fields.insert("hostname".to_string(), ctx.hostname.clone());
        Some(fields)
    }
}
