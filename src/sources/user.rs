use std::collections::HashMap;

use super::{Source, SourceContext};

pub struct User;

impl Source for User {
    fn kind(&self) -> &'static str {
        "user"
    }
    fn default_name(&self) -> &'static str {
        "{user}"
    }
    fn default_email(&self) -> &'static str {
        "{user}@users.local"
    }

    fn fields(&self, ctx: &SourceContext) -> Option<HashMap<String, String>> {
        if ctx.username.is_empty() {
            return None;
        }
        let mut fields = HashMap::new();
        fields.insert("user".to_string(), ctx.username.clone());
        Some(fields)
    }
}
