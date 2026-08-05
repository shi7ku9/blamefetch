use std::collections::HashMap;
use std::path::Path;

use crate::util::format_bytes;
use sysinfo::Disks;

use super::{Source, SourceContext};

pub struct Disk;

impl Source for Disk {
    fn kind(&self) -> &'static str {
        "disk"
    }
    fn default_name(&self) -> &'static str {
        "{used} / {total}"
    }
    fn default_email(&self) -> &'static str {
        "disk@system.invalid"
    }

    fn fields(&self, _ctx: &SourceContext) -> Option<HashMap<String, String>> {
        let disks = Disks::new_with_refreshed_list(); // 0.39: System::disks() removed; use Disks::new_with_refreshed_list()
        let list = disks.list();
        let disk = list
            .iter()
            .find(|d| d.mount_point() == Path::new("/"))
            .or_else(|| list.first())?;
        let total = disk.total_space();
        if total == 0 {
            return None;
        }
        let used = total.saturating_sub(disk.available_space());
        let mut fields = HashMap::new();
        fields.insert("used".to_string(), format_bytes(used));
        fields.insert("total".to_string(), format_bytes(total));
        Some(fields)
    }
}
