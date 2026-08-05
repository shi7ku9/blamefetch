use std::collections::HashMap;

use crate::util::format_bytes;

use super::{Source, SourceContext};

pub struct Memory;

impl Source for Memory {
    fn kind(&self) -> &'static str {
        "memory"
    }
    fn default_name(&self) -> &'static str {
        "{used} / {total}"
    }
    fn default_email(&self) -> &'static str {
        "memory@system.invalid"
    }

    fn fields(&self, ctx: &SourceContext) -> Option<HashMap<String, String>> {
        let total = ctx.sys.total_memory();
        if total == 0 {
            return None;
        }
        let mut fields = HashMap::new();
        fields.insert("used".to_string(), format_bytes(ctx.sys.used_memory()));
        fields.insert("total".to_string(), format_bytes(total));
        Some(fields)
    }
}
