use std::collections::HashMap;

use super::{Source, SourceContext};

pub struct Cpu;

impl Source for Cpu {
    fn kind(&self) -> &'static str {
        "cpu"
    }
    fn default_name(&self) -> &'static str {
        "{model} ({cores} threads) @ {freq}"
    }
    fn default_email(&self) -> &'static str {
        "cpu@system.invalid"
    }

    fn fields(&self, ctx: &SourceContext) -> Option<HashMap<String, String>> {
        let cpus = ctx.sys.cpus();
        let first = cpus.first()?;
        let max_mhz = cpus.iter().map(|c| c.frequency()).max().unwrap_or(0);
        let mut fields = HashMap::new();
        fields.insert("model".to_string(), first.brand().to_string());
        fields.insert("cores".to_string(), cpus.len().to_string());
        fields.insert(
            "freq".to_string(),
            format!("{:.2} GHz", max_mhz as f64 / 1000.0),
        );
        Some(fields)
    }
}
