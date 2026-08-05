use std::collections::HashMap;

use super::{Source, SourceContext};

pub struct Host;

impl Source for Host {
    fn kind(&self) -> &'static str {
        "host"
    }
    fn default_name(&self) -> &'static str {
        "{product}"
    }
    fn default_email(&self) -> &'static str {
        "host@system.invalid"
    }

    fn fields(&self, _ctx: &SourceContext) -> Option<HashMap<String, String>> {
        let product = std::fs::read_to_string("/sys/class/dmi/id/product_name").ok()?;
        let product = product.trim();
        if product.is_empty() {
            return None;
        }
        let mut fields = HashMap::new();
        fields.insert("product".to_string(), product.to_string());
        Some(fields)
    }
}
