use std::collections::HashMap;

use crate::util::format_duration;

use super::{Source, SourceContext};

pub struct Uptime;

impl Source for Uptime {
    fn kind(&self) -> &'static str {
        "uptime"
    }
    fn default_name(&self) -> &'static str {
        "{duration}"
    }
    fn default_email(&self) -> &'static str {
        "uptime@system.invalid"
    }

    fn fields(&self, _ctx: &SourceContext) -> Option<HashMap<String, String>> {
        let mut fields = HashMap::new();
        fields.insert(
            "duration".to_string(),
            format_duration(sysinfo::System::uptime()),
        ); // 0.39: uptime() is an associated fn
        Some(fields)
    }
}
