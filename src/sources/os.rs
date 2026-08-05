use std::collections::HashMap;

use super::{Source, SourceContext};

pub struct Os;

impl Source for Os {
    fn kind(&self) -> &'static str {
        "os"
    }
    fn default_name(&self) -> &'static str {
        "{name} {version}"
    }
    fn default_email(&self) -> &'static str {
        "os@system.invalid"
    }

    fn fields(&self, ctx: &SourceContext) -> Option<HashMap<String, String>> {
        let mut fields = HashMap::new();
        fields.insert("name".to_string(), ctx.os_info.os_type().to_string());
        fields.insert("version".to_string(), ctx.os_info.version().to_string());
        Some(fields)
    }
}
